//! In-process integration test for wl_shm buffer commits populating renderer surface state.

use std::io::Write;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::components::{SurfaceGeometry, WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::CompositorConfig;
use nekoland_protocol::{ProtocolServerState, ProtocolSurfaceRegistry};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::utils::{Logical, Size};
use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

/// Width of the wl_shm buffer committed by the helper client.
const TEST_BUFFER_WIDTH: u32 = 96;
/// Height of the wl_shm buffer committed by the helper client.
const TEST_BUFFER_HEIGHT: u32 = 64;
/// Extra dwell time after attach so renderer state extraction can catch up.
const CLIENT_POST_ATTACH_HOLD: Duration = Duration::from_millis(300);

/// Summary returned by the helper SHM client.
#[derive(Debug)]
struct ShmClientSummary {
    globals: Vec<String>,
    configure_serial: u32,
}

/// Helper Wayland client state used to create one toplevel and attach a wl_shm buffer.
#[derive(Debug, Default)]
struct ShmClientState {
    globals: Vec<String>,
    base_surface: Option<wl_surface::WlSurface>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    _toplevel: Option<xdg_toplevel::XdgToplevel>,
    _pool: Option<wl_shm_pool::WlShmPool>,
    buffer: Option<wl_buffer::WlBuffer>,
    _backing_file: Option<std::fs::File>,
    /// Last configure serial seen for the helper toplevel.
    configure_serial: Option<u32>,
    /// Whether the wl_shm buffer has already been attached and committed.
    buffer_attached: bool,
}

/// Verifies that committing a wl_shm buffer initializes Smithay renderer surface state and the
/// corresponding window geometry.
#[test]
fn shm_buffer_commit_populates_renderer_surface_state() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-shm-runtime");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });
    {
        let Some(mut config) = app.inner_mut().world_mut().get_resource_mut::<CompositorConfig>()
        else {
            panic!("runtime config should be initialized before tests mutate it");
        };
        config.default_layout = nekoland_ecs::resources::DefaultLayout::Floating;
    }

    let socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<ProtocolServerState>() else {
            panic!("protocol server state should be available immediately after build");
        };

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!(
                    "skipping in-process shm renderer test in restricted environment: {error}"
                );
                return;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let client_thread = thread::spawn(move || run_shm_client(&socket_path));
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!(
                    "skipping in-process shm renderer test in restricted environment: {reason}"
                );
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("in-process shm client failed: {reason}");
            }
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.configure_serial > 0, "client should ack a configure");

    let (surface_id, geometry, wl_surface) = {
        let world = app.inner_mut().world_mut();
        let mut windows = world.query_filtered::<
            (&WlSurfaceHandle, &SurfaceGeometry),
            bevy_ecs::query::With<XdgWindow>,
        >();
        let window =
            windows.iter(world).next().map(|(surface, geometry)| (surface.id, geometry.clone()));
        let Some((surface_id, geometry)) = window else {
            panic!("shm client should produce an XdgWindow entity");
        };
        let Some(registry) = world.get_non_send_resource::<ProtocolSurfaceRegistry>() else {
            panic!("protocol surface registry should be initialized");
        };
        let wl_surface = registry.surface(surface_id).cloned().unwrap_or_else(|| {
            panic!("tracked window surface should remain available in protocol registry")
        });
        (surface_id, geometry, wl_surface)
    };

    let renderer_state = with_renderer_surface_state(&wl_surface, |state| {
        (state.buffer().is_some(), state.buffer_size(), state.surface_size(), state.buffer_scale())
    });
    let Some((buffer_present, buffer_size, surface_size, buffer_scale)) = renderer_state else {
        panic!("wl_shm commit should initialize renderer surface state");
    };

    assert!(buffer_present, "renderer surface state should retain the attached shm buffer");
    assert_eq!(
        buffer_size,
        Some(Size::<i32, Logical>::from((TEST_BUFFER_WIDTH as i32, TEST_BUFFER_HEIGHT as i32))),
        "renderer surface state should expose the attached buffer size for surface {surface_id}"
    );
    assert_eq!(
        surface_size,
        Some(Size::<i32, Logical>::from((TEST_BUFFER_WIDTH as i32, TEST_BUFFER_HEIGHT as i32))),
        "surface view should match the shm buffer size for surface {surface_id}"
    );
    assert_eq!(buffer_scale, 1, "shm buffer should keep the default scale");
    assert_eq!(
        geometry.width, TEST_BUFFER_WIDTH,
        "new window geometry should be initialized from the committed shm buffer width"
    );
    assert_eq!(
        geometry.height, TEST_BUFFER_HEIGHT,
        "new window geometry should be initialized from the committed shm buffer height"
    );
}

/// Runs the helper SHM client until a buffer has been attached.
fn run_shm_client(socket_path: &Path) -> Result<ShmClientSummary, common::TestControl> {
    let stream = std::os::unix::net::UnixStream::connect(socket_path)
        .map_err(|error| common::TestControl::Fail(error.to_string()))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| common::TestControl::Fail(format!("set_read_timeout failed: {error}")))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| common::TestControl::Fail(format!("set_write_timeout failed: {error}")))?;

    let conn = Connection::from_socket(stream)
        .map_err(|error| common::TestControl::Fail(format!("from_socket failed: {error}")))?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = ShmClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while !state.buffer_attached {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for wl_shm surface attach".to_owned(),
            ));
        }
    }

    event_queue.flush().map_err(|error| {
        common::TestControl::Fail(format!("flush after shm attach failed: {error}"))
    })?;
    thread::sleep(CLIENT_POST_ATTACH_HOLD);

    Ok(ShmClientSummary {
        globals: state.globals,
        configure_serial: state.configure_serial.ok_or_else(|| {
            common::TestControl::Fail("client never received xdg_surface.configure".to_owned())
        })?,
    })
}

/// Performs one read/dispatch cycle for the helper SHM client.
fn dispatch_client_once(
    event_queue: &mut EventQueue<ShmClientState>,
    state: &mut ShmClientState,
) -> Result<(), common::TestControl> {
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
    })?;
    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

    let Some(read_guard) = event_queue.prepare_read() else {
        return Ok(());
    };

    read_guard.read().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
    })?;
    Ok(())
}

/// Creates a simple SHM buffer filled with deterministic pixel data for the scenario.
fn create_test_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<ShmClientState>,
) -> Result<(std::fs::File, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer), common::TestControl> {
    let stride = TEST_BUFFER_WIDTH * 4;
    let file_size = stride * TEST_BUFFER_HEIGHT;
    let mut file = tempfile().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    let mut pixels = vec![0_u8; file_size as usize];
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0x66, 0x99, 0xcc, 0x00]);
    }
    file.write_all(&pixels).map_err(|error| {
        common::TestControl::Fail(format!("write shm backing file failed: {error}"))
    })?;
    file.flush().map_err(|error| {
        common::TestControl::Fail(format!("flush shm backing file failed: {error}"))
    })?;

    let pool = shm.create_pool(file.as_fd(), file_size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        TEST_BUFFER_WIDTH as i32,
        TEST_BUFFER_HEIGHT as i32,
        stride as i32,
        wl_shm::Format::Xrgb8888,
        qh,
        (),
    );

    Ok((file, pool, buffer))
}

/// Returns the default config path used by this integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

impl ShmClientState {
    /// Creates the helper toplevel once the base surface and XDG shell are both ready.
    fn maybe_init_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_none() || self.wm_base.is_none() || self.xdg_surface.is_some() {
            return;
        }

        let (Some(surface), Some(wm_base)) = (self.base_surface.as_ref(), self.wm_base.as_ref())
        else {
            return;
        };
        let xdg_surface = wm_base.get_xdg_surface(surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.xdg_surface = Some(xdg_surface);
        self._toplevel = Some(toplevel);
    }

    /// Create the wl_shm buffer lazily once the shm global is available.
    fn maybe_init_buffer(&mut self, qh: &QueueHandle<Self>) -> Result<(), common::TestControl> {
        if self.shm.is_none() || self.buffer.is_some() {
            return Ok(());
        }

        let Some(shm) = self.shm.as_ref() else {
            return Ok(());
        };
        let (file, pool, buffer) = create_test_buffer(shm, qh)?;
        self._backing_file = Some(file);
        self._pool = Some(pool);
        self.buffer = Some(buffer);
        self.maybe_attach_buffer();
        Ok(())
    }

    /// Attach the prepared buffer after the first configure so the surface becomes renderable.
    fn maybe_attach_buffer(&mut self) {
        if self.buffer_attached || self.configure_serial.is_none() {
            return;
        }

        let Some(surface) = self.base_surface.as_ref() else {
            return;
        };
        let Some(buffer) = self.buffer.as_ref() else {
            return;
        };

        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, TEST_BUFFER_WIDTH as i32, TEST_BUFFER_HEIGHT as i32);
        surface.commit();
        self.buffer_attached = true;
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for ShmClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            state.globals.push(interface.clone());

            match interface.as_str() {
                "wl_compositor" => {
                    let compositor =
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ());
                    state.base_surface = Some(compositor.create_surface(qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                    if let Err(error) = state.maybe_init_buffer(qh) {
                        panic!("failed to initialize shm buffer: {error:?}");
                    }
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ShmClientState {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for ShmClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            state.configure_serial = Some(serial);
            xdg_surface.ack_configure(serial);
            if let Err(error) = state.maybe_init_buffer(qh) {
                panic!("failed to create wl_shm buffer after configure: {error:?}");
            }
            state.maybe_attach_buffer();
        }
    }
}

delegate_noop!(ShmClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(ShmClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(ShmClientState: ignore wl_shm::WlShm);
delegate_noop!(ShmClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ShmClientState: ignore wl_surface::WlSurface);
delegate_noop!(ShmClientState: ignore xdg_toplevel::XdgToplevel);
