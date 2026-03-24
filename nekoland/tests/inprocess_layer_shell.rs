//! In-process integration test for layer-shell surfaces reaching ECS state, render plans, and
//! work-area updates.

use std::io::Write;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::{NekolandApp, RunLoopSettings};
use nekoland_ecs::components::{
    LayerShellSurface, OutputProperties, SurfaceGeometry, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{CompiledOutputFrames, RenderPlan, RenderPlanItem, WorkArea};
use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

mod common;

const TEST_LAYER_WIDTH: u32 = 320;
const TEST_LAYER_HEIGHT: u32 = 32;
const TEST_EXCLUSIVE_PANEL_HEIGHT: u32 = 48;
const CLIENT_POST_ATTACH_HOLD: Duration = Duration::from_secs(1);

/// Options that control the helper layer-shell client scenario.
#[derive(Debug, Clone, Copy)]
struct LayerClientOptions {
    namespace: &'static str,
    requested_width: u32,
    requested_height: u32,
    buffer_width: u32,
    buffer_height: u32,
    anchor_top: bool,
    anchor_bottom: bool,
    anchor_left: bool,
    anchor_right: bool,
    exclusive_zone: Option<i32>,
    bind_output: bool,
}

impl LayerClientOptions {
    /// Standard top-left panel with a fixed requested size.
    fn standard_panel() -> Self {
        Self {
            namespace: "nekoland-panel",
            requested_width: TEST_LAYER_WIDTH,
            requested_height: TEST_LAYER_HEIGHT,
            buffer_width: TEST_LAYER_WIDTH,
            buffer_height: TEST_LAYER_HEIGHT,
            anchor_top: true,
            anchor_bottom: false,
            anchor_left: true,
            anchor_right: false,
            exclusive_zone: None,
            bind_output: false,
        }
    }

    /// Full-width top panel that also reserves exclusive work-area space.
    fn exclusive_top_panel() -> Self {
        Self {
            namespace: "nekoland-exclusive-panel",
            requested_width: 0,
            requested_height: TEST_EXCLUSIVE_PANEL_HEIGHT,
            buffer_width: TEST_LAYER_WIDTH,
            buffer_height: TEST_EXCLUSIVE_PANEL_HEIGHT,
            anchor_top: true,
            anchor_bottom: false,
            anchor_left: true,
            anchor_right: true,
            exclusive_zone: Some(TEST_EXCLUSIVE_PANEL_HEIGHT as i32),
            bind_output: false,
        }
    }

    fn bound_standard_panel() -> Self {
        Self { bind_output: true, ..Self::standard_panel() }
    }

    /// Converts the booleans in this helper struct into the Wayland layer-shell anchor bitflags.
    fn anchor(self) -> zwlr_layer_surface_v1::Anchor {
        let mut anchor = zwlr_layer_surface_v1::Anchor::empty();
        if self.anchor_top {
            anchor |= zwlr_layer_surface_v1::Anchor::Top;
        }
        if self.anchor_bottom {
            anchor |= zwlr_layer_surface_v1::Anchor::Bottom;
        }
        if self.anchor_left {
            anchor |= zwlr_layer_surface_v1::Anchor::Left;
        }
        if self.anchor_right {
            anchor |= zwlr_layer_surface_v1::Anchor::Right;
        }
        anchor
    }
}

/// Summary returned by the helper layer-shell client.
#[derive(Debug)]
struct LayerClientSummary {
    globals: Vec<String>,
    configure_serial: u32,
}

/// Helper Wayland client state for the layer-shell scenario.
#[derive(Debug)]
struct LayerClientState {
    options: LayerClientOptions,
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    output: Option<wl_output::WlOutput>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    _pool: Option<wl_shm_pool::WlShmPool>,
    _buffer: Option<wl_buffer::WlBuffer>,
    _backing_file: Option<std::fs::File>,
    configure_serial: Option<u32>,
    buffer_attached: bool,
}

impl LayerClientState {
    /// Initializes helper state for one layer-shell client run.
    fn new(options: LayerClientOptions) -> Self {
        Self {
            options,
            globals: Vec::new(),
            compositor: None,
            shm: None,
            layer_shell: None,
            output: None,
            surface: None,
            layer_surface: None,
            _pool: None,
            _buffer: None,
            _backing_file: None,
            configure_serial: None,
            buffer_attached: false,
        }
    }

    /// Creates the layer surface once both `wl_compositor` and `zwlr_layer_shell_v1` are bound.
    fn maybe_create_layer_surface(&mut self, qh: &QueueHandle<Self>) {
        if self.surface.is_some()
            || self.compositor.is_none()
            || self.layer_shell.is_none()
            || (self.options.bind_output && self.output.is_none())
        {
            return;
        }

        let (Some(compositor), Some(layer_shell)) =
            (self.compositor.as_ref(), self.layer_shell.as_ref())
        else {
            return;
        };

        let surface = compositor.create_surface(qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            self.output.as_ref(),
            zwlr_layer_shell_v1::Layer::Top,
            self.options.namespace.to_owned(),
            qh,
            (),
        );
        layer_surface.set_anchor(self.options.anchor());
        layer_surface.set_size(self.options.requested_width, self.options.requested_height);
        if let Some(exclusive_zone) = self.options.exclusive_zone {
            layer_surface.set_exclusive_zone(exclusive_zone);
        }
        surface.commit();

        self.surface = Some(surface);
        self.layer_surface = Some(layer_surface);
    }

    /// Lazily allocates and attaches a simple SHM buffer after the first configure arrives.
    fn maybe_attach_buffer(&mut self, qh: &QueueHandle<Self>) -> Result<(), common::TestControl> {
        if self.buffer_attached || self.configure_serial.is_none() {
            return Ok(());
        }

        if self._buffer.is_none() {
            let shm = self
                .shm
                .as_ref()
                .ok_or_else(|| common::TestControl::Fail("wl_shm global missing".to_owned()))?;
            let (file, pool, buffer) = create_test_buffer(shm, qh, self.options)?;
            self._backing_file = Some(file);
            self._pool = Some(pool);
            self._buffer = Some(buffer);
        }

        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| common::TestControl::Fail("layer surface missing".to_owned()))?;
        let buffer = self
            ._buffer
            .as_ref()
            .ok_or_else(|| common::TestControl::Fail("layer shm buffer missing".to_owned()))?;
        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, self.options.buffer_width as i32, self.options.buffer_height as i32);
        surface.commit();
        self.buffer_attached = true;
        Ok(())
    }
}

/// Verifies that a mapped layer-shell surface appears in ECS and the render plan.
#[test]
fn layer_shell_surface_reaches_ecs_and_render_plan() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-layer-shell-runtime");
    let config_path =
        common::write_default_config_with_xwayland_disabled(&runtime_dir.path, "layer-shell.toml");
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });

    let socket_path = match protocol_socket_path(&app, &runtime_dir.path) {
        Ok(path) => path,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping layer-shell test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("protocol startup failed before run: {reason}");
        }
    };

    let client_thread = thread::spawn(move || {
        run_layer_shell_client(&socket_path, LayerClientOptions::standard_panel())
    });
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping layer-shell test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("layer-shell client failed: {reason}");
            }
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.configure_serial > 0, "layer-shell client should ack a configure");

    let (surface_id, geometry, namespace, render_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let mut layers = world.query::<(&WlSurfaceHandle, &SurfaceGeometry, &LayerShellSurface)>();
        let layer_row = layers.iter(world).next().map(|(surface, geometry, layer_surface)| {
            (surface.id, geometry.clone(), layer_surface.namespace.clone())
        });
        let Some((surface_id, geometry, namespace)) = layer_row else {
            panic!("layer-shell client should create a layer entity");
        };
        let render_plan = if let Some(compiled) = world.get_resource::<CompiledOutputFrames>() {
            &compiled.render_plan
        } else if let Some(render_plan) = world.get_resource::<RenderPlan>() {
            render_plan
        } else {
            panic!("render plan should be available");
        };
        let render_surface_ids = render_plan
            .outputs
            .values()
            .flat_map(|output_plan| output_plan.iter_ordered())
            .filter_map(|item| match item {
                RenderPlanItem::Surface(item) => Some(item.surface_id),
                RenderPlanItem::Quad(_)
                | RenderPlanItem::Backdrop(_)
                | RenderPlanItem::Cursor(_) => None,
            })
            .collect::<Vec<_>>();
        (surface_id, geometry, namespace, render_surface_ids)
    };

    assert_eq!(namespace, LayerClientOptions::standard_panel().namespace);
    assert_eq!(geometry.x, 0, "top-left anchored layer should hug the left edge");
    assert_eq!(geometry.y, 0, "top-left anchored layer should hug the top edge");
    assert_eq!(geometry.width, TEST_LAYER_WIDTH);
    assert_eq!(geometry.height, TEST_LAYER_HEIGHT);
    assert!(
        render_surface_ids.contains(&surface_id),
        "render plan should contain the mapped layer surface: {render_surface_ids:?}"
    );
}

#[test]
fn output_bound_layer_shell_surface_still_maps_when_binding_real_output() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-layer-output-bind-runtime");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "layer-output-bind.toml",
    );
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });

    let socket_path = match protocol_socket_path(&app, &runtime_dir.path) {
        Ok(path) => path,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping output-bound layer-shell test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("protocol startup failed before run: {reason}");
        }
    };

    let client_thread = thread::spawn(move || {
        run_layer_shell_client(&socket_path, LayerClientOptions::bound_standard_panel())
    });
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!(
                    "skipping output-bound layer-shell test in restricted environment: {reason}"
                );
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("layer-shell client failed: {reason}");
            }
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.configure_serial > 0, "layer-shell client should ack a configure");

    let world = app.inner_mut().world_mut();
    let output_exists = world.query::<&OutputProperties>().iter(world).next().is_some();
    let mut layers = world.query::<(&WlSurfaceHandle, &SurfaceGeometry, &LayerShellSurface)>();
    let Some((_, geometry, _)) = layers.iter(world).next() else {
        panic!("output-bound layer-shell client should create a layer entity");
    };

    assert!(output_exists, "the compositor should expose at least one real output");
    assert_eq!(geometry.x, 0);
    assert_eq!(geometry.y, 0);
}

#[test]
fn exclusive_top_layer_reserves_work_area_for_new_windows() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-layer-work-area-runtime");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "layer-work-area.toml",
    );
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(128),
    });

    let socket_path = match protocol_socket_path(&app, &runtime_dir.path) {
        Ok(path) => path,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping layer work-area test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("protocol startup failed before run: {reason}");
        }
    };

    let layer_socket_path = socket_path.clone();
    let layer_thread = thread::spawn(move || {
        run_layer_shell_client(&layer_socket_path, LayerClientOptions::exclusive_top_panel())
    });
    let xdg_socket_path = socket_path.clone();
    let xdg_thread = thread::spawn(move || {
        common::run_xdg_client_with_hold(&xdg_socket_path, CLIENT_POST_ATTACH_HOLD)
    });

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let layer_summary = match layer_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping layer work-area test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("layer-shell client failed: {reason}");
            }
        },
        Err(_) => panic!("layer client thread should exit cleanly"),
    };
    let xdg_summary = match xdg_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping layer work-area test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("xdg client failed: {reason}");
            }
        },
        Err(_) => panic!("xdg client thread should exit cleanly"),
    };

    common::assert_globals_present(&layer_summary.globals);
    common::assert_globals_present(&xdg_summary.globals);
    assert!(layer_summary.configure_serial > 0, "layer-shell client should ack a configure");
    assert!(xdg_summary.configure_serial > 0, "xdg client should ack a configure");

    let (output, work_area, layer_geometry, layer_namespace, window_geometry) = {
        let world = app.inner_mut().world_mut();
        let output = world.query::<&OutputProperties>().iter(world).next().cloned();
        let Some(output) = output else {
            panic!("output properties should exist");
        };
        let Some(work_area) = world.get_resource::<WorkArea>() else {
            panic!("work area resource should exist");
        };
        let work_area = *work_area;
        let mut layers = world.query::<(&SurfaceGeometry, &LayerShellSurface)>();
        let layer_state = layers
            .iter(world)
            .find(|(_, layer)| {
                layer.namespace == LayerClientOptions::exclusive_top_panel().namespace
            })
            .map(|(geometry, layer)| (geometry.clone(), layer.namespace.clone()));
        let Some((layer_geometry, layer_namespace)) = layer_state else {
            panic!("exclusive layer surface should exist");
        };
        let window_geometry = world.query::<(&SurfaceGeometry, &XdgWindow)>().iter(world).next();
        let Some(window_geometry) = window_geometry.map(|(geometry, _)| geometry.clone()) else {
            panic!("xdg window should exist");
        };
        (output, work_area, layer_geometry, layer_namespace, window_geometry)
    };

    assert_eq!(layer_namespace, LayerClientOptions::exclusive_top_panel().namespace);
    assert_eq!(layer_geometry.x, 0);
    assert_eq!(layer_geometry.y, 0);
    assert_eq!(layer_geometry.width, output.width);
    assert_eq!(layer_geometry.height, TEST_EXCLUSIVE_PANEL_HEIGHT);
    assert_eq!(work_area.x, 0);
    assert_eq!(work_area.y, TEST_EXCLUSIVE_PANEL_HEIGHT as i32);
    assert_eq!(work_area.width, output.width);
    assert_eq!(work_area.height, output.height.saturating_sub(TEST_EXCLUSIVE_PANEL_HEIGHT));
    assert!(
        window_geometry.y >= work_area.y,
        "new windows should not be placed underneath exclusive top layers: {window_geometry:?} work_area={work_area:?}"
    );
}

/// Resolves the protocol socket path or classifies startup failure as skip/fail for the test.
fn protocol_socket_path(
    app: &NekolandApp,
    runtime_dir: &Path,
) -> Result<PathBuf, common::TestControl> {
    let server_state = common::protocol_server_state(app);

    match (&server_state.socket_name, &server_state.startup_error) {
        (Some(socket_name), _) => Ok(runtime_dir.join(socket_name)),
        (None, Some(error)) if error.contains("Operation not permitted") => {
            Err(common::TestControl::Skip(error.clone()))
        }
        (None, Some(error)) => Err(common::TestControl::Fail(error.clone())),
        (None, None) => Err(common::TestControl::Fail(
            "protocol startup produced neither socket nor error".to_owned(),
        )),
    }
}

/// Runs the helper layer-shell client until it receives a configure and attaches its SHM buffer.
fn run_layer_shell_client(
    socket_path: &Path,
    options: LayerClientOptions,
) -> Result<LayerClientSummary, common::TestControl> {
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

    let mut state = LayerClientState::new(options);
    let deadline = Instant::now() + Duration::from_secs(2);

    while Instant::now() < deadline {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if state.buffer_attached {
            break;
        }
    }

    if !state.buffer_attached {
        return Err(common::TestControl::Fail(
            "timed out waiting for layer-shell buffer attach".to_owned(),
        ));
    }

    event_queue.flush().map_err(|error| {
        common::TestControl::Fail(format!("flush after layer attach failed: {error}"))
    })?;
    thread::sleep(CLIENT_POST_ATTACH_HOLD);

    Ok(LayerClientSummary {
        globals: state.globals,
        configure_serial: state.configure_serial.ok_or_else(|| {
            common::TestControl::Fail("layer-shell client never received configure".to_owned())
        })?,
    })
}

/// Performs one read/dispatch cycle for the helper layer-shell client.
fn dispatch_client_once(
    event_queue: &mut EventQueue<LayerClientState>,
    state: &mut LayerClientState,
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

/// Creates a small SHM buffer with deterministic pixel data for the helper layer-shell client.
fn create_test_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<LayerClientState>,
    options: LayerClientOptions,
) -> Result<(std::fs::File, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer), common::TestControl> {
    let stride = options.buffer_width * 4;
    let file_size = stride * options.buffer_height;
    let mut file = tempfile().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    let mut pixels = vec![0_u8; file_size as usize];
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0xaa, 0xbb, 0xcc, 0x00]);
    }
    file.write_all(&pixels).map_err(|error| {
        common::TestControl::Fail(format!("write layer shm file failed: {error}"))
    })?;
    file.flush().map_err(|error| {
        common::TestControl::Fail(format!("flush layer shm file failed: {error}"))
    })?;

    let pool = shm.create_pool(file.as_fd(), file_size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        options.buffer_width as i32,
        options.buffer_height as i32,
        stride as i32,
        wl_shm::Format::Xrgb8888,
        qh,
        (),
    );
    Ok((file, pool, buffer))
}

impl Dispatch<wl_registry::WlRegistry, ()> for LayerClientState {
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
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()));
                    state.maybe_create_layer_surface(qh);
                }
                "wl_output" => {
                    state.output =
                        Some(registry.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ()));
                    state.maybe_create_layer_surface(qh);
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell =
                        Some(registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                            name,
                            4,
                            qh,
                            (),
                        ));
                    state.maybe_create_layer_surface(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for LayerClientState {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            state.configure_serial = Some(serial);
            layer_surface.ack_configure(serial);
            if let Err(error) = state.maybe_attach_buffer(qh) {
                panic!("failed to attach layer-shell shm buffer: {error:?}");
            }
        }
    }
}

delegate_noop!(LayerClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(LayerClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(LayerClientState: ignore wl_output::WlOutput);
delegate_noop!(LayerClientState: ignore wl_shm::WlShm);
delegate_noop!(LayerClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(LayerClientState: ignore wl_surface::WlSurface);
delegate_noop!(LayerClientState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
