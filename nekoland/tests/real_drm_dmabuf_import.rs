//! Real-backend integration test for dma-buf import on DRM/GBM-capable systems.
//!
//! This test is intentionally opt-in. It runs only when
//! `NEKOLAND_RUN_REAL_DRM_IMPORT_TEST=1` is present, because it requires:
//! - a real DRM backend environment
//! - a usable render node for GBM allocation
//! - enough permissions for the compositor to acquire the DRM session
//!
//! When enabled, the helper client creates a GBM-backed dma-buf, submits it via
//! `zwp_linux_dmabuf_v1`, and the test verifies that the compositor carries the
//! surface through wayland snapshots, render prepared imports, and backend
//! present audit without falling back to `Unsupported`.

use std::fs::{self, OpenOptions};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_config::resources::{CompositorConfig, DefaultLayout};
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::components::{WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{
    CompiledOutputFrames, PlatformSurfaceBufferSource, PlatformSurfaceImportStrategy,
    PreparedSurfaceImportStrategy, ShellRenderInput, WaylandFeedback, WaylandIngress,
};
use smithay::backend::allocator::Buffer as AllocatorBuffer;
use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::reexports::drm::buffer::Buffer as DrmBuffer;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

const TEST_WIDTH: u32 = 128;
const TEST_HEIGHT: u32 = 96;
const POST_ATTACH_HOLD: Duration = Duration::from_millis(300);
const ENABLE_ENV: &str = "NEKOLAND_RUN_REAL_DRM_IMPORT_TEST";
const RENDER_NODE_ENV: &str = "NEKOLAND_TEST_RENDER_NODE";

#[derive(Debug)]
struct DmabufClientSummary {
    globals: Vec<String>,
    configure_serial: u32,
}

#[derive(Debug, Default)]
struct DmabufClientState {
    globals: Vec<String>,
    base_surface: Option<wl_surface::WlSurface>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    _toplevel: Option<xdg_toplevel::XdgToplevel>,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    params: Option<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1>,
    buffer: Option<wl_buffer::WlBuffer>,
    configure_serial: Option<u32>,
    buffer_requested: bool,
    buffer_attached: bool,
    import_failed: bool,
}

#[test]
fn real_drm_backend_imports_dmabuf_surface_end_to_end() {
    if std::env::var_os(ENABLE_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        eprintln!(
            "skipping real DRM dma-buf import test; set {ENABLE_ENV}=1 to enable hardware verification"
        );
        return;
    }

    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-real-drm-dmabuf-runtime");
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "drm");
    let render_node_path = match discover_render_node() {
        Ok(path) => path,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping real DRM dma-buf import test: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => panic!("{reason}"),
    };

    let config_path = common::default_workspace_config_path();
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(192),
    });
    {
        let Some(mut config) = app.inner_mut().world_mut().get_resource_mut::<CompositorConfig>()
        else {
            panic!("runtime config should be initialized before tests mutate it");
        };
        config.default_layout = DefaultLayout::Floating;
    }

    let socket_path = match common::protocol_socket_path(&app, &runtime_dir.path) {
        Ok(path) => path,
        Err(error) if looks_like_restricted_runtime_error(&error) => {
            eprintln!("skipping real DRM dma-buf import test in restricted environment: {error}");
            return;
        }
        Err(error) => panic!("protocol startup failed before run: {error}"),
    };

    let client_thread =
        thread::spawn(move || run_dmabuf_client(&socket_path, &render_node_path));

    if let Err(error) = app.run() {
        let message = error.to_string();
        if looks_like_restricted_runtime_error(&message)
            || message.contains("DRM backend unavailable")
            || message.contains("no primary DRM node found")
        {
            eprintln!("skipping real DRM dma-buf import test in restricted environment: {message}");
            return;
        }
        panic!("nekoland app should complete the configured frame budget: {message}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping real DRM dma-buf import test: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => panic!("real DRM dma-buf client failed: {reason}"),
        },
        Err(_) => panic!("real DRM dma-buf client thread should exit cleanly"),
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.configure_serial > 0, "client should ack a configure");

    let surface_id = {
        let world = app.inner_mut().world_mut();
        let mut windows = world.query_filtered::<
            &WlSurfaceHandle,
            bevy_ecs::query::With<XdgWindow>,
        >();
        let Some(surface) = windows.iter(world).next() else {
            panic!("dma-buf client should produce an XdgWindow entity");
        };
        surface.id
    };

    let ingress = app.inner().world().resource::<WaylandIngress>();
    assert!(
        ingress.import_capabilities.dmabuf_importable,
        "real DRM backend should advertise dma-buf import capability"
    );
    let surface_snapshot = ingress
        .surface_snapshots
        .surfaces
        .get(&surface_id)
        .unwrap_or_else(|| panic!("surface snapshot should exist for imported dma-buf surface {surface_id}"));
    assert_eq!(surface_snapshot.buffer_source, PlatformSurfaceBufferSource::DmaBuf);
    assert_ne!(surface_snapshot.import_strategy, PlatformSurfaceImportStrategy::Unsupported);
    assert!(
        surface_snapshot.dmabuf_format.is_some(),
        "real dma-buf surface should export format metadata through WaylandIngress"
    );

    let compiled_frames = app.inner().world().resource::<CompiledOutputFrames>();
    let prepared_import = compiled_frames
        .outputs
        .values()
        .filter_map(|frame| frame.gpu_prep.as_ref())
        .find_map(|gpu_prep| gpu_prep.surface_imports.get(&surface_id))
        .unwrap_or_else(|| panic!("compiled frame should carry prepared import for surface {surface_id}"));
    assert_eq!(prepared_import.descriptor.buffer_source, PlatformSurfaceBufferSource::DmaBuf);
    assert_ne!(prepared_import.strategy, PreparedSurfaceImportStrategy::Unsupported);

    let present_audit = &app.inner().world().resource::<WaylandFeedback>().present_audit;
    if present_audit.outputs.is_empty() {
        eprintln!("skipping real DRM dma-buf import test: DRM backend reported no active present outputs");
        return;
    }
    assert!(
        present_audit
            .outputs
            .values()
            .any(|output| output.elements.iter().any(|element| element.surface_id == surface_id)),
        "present audit should include the imported dma-buf surface"
    );

    let shell_render_input = app.inner().world().resource::<ShellRenderInput>();
    assert!(
        shell_render_input.surface_presentation.surfaces.contains_key(&surface_id),
        "shell render mailbox should track the imported dma-buf surface presentation"
    );
}

fn discover_render_node() -> Result<PathBuf, common::TestControl> {
    if let Some(path) = std::env::var_os(RENDER_NODE_ENV).map(PathBuf::from) {
        if path.exists() {
            return Ok(path);
        }
        return Err(common::TestControl::Skip(format!(
            "{RENDER_NODE_ENV} points to missing render node {}",
            path.display()
        )));
    }

    let mut entries = fs::read_dir("/dev/dri")
        .map_err(|error| common::TestControl::Skip(format!("unable to read /dev/dri: {error}")))?;
    if let Some(path) = entries.find_map(|entry| {
        let entry = entry.ok()?;
        let file_name = entry.file_name();
        file_name.to_string_lossy().starts_with("renderD").then(|| entry.path())
    }) {
        return Ok(path);
    }

    Err(common::TestControl::Skip(
        "no DRM render node found under /dev/dri".to_owned(),
    ))
}

fn run_dmabuf_client(
    socket_path: &Path,
    render_node_path: &Path,
) -> Result<DmabufClientSummary, common::TestControl> {
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

    let mut state = DmabufClientState::default();
    let deadline = Instant::now() + Duration::from_secs(4);

    while !state.buffer_attached && !state.import_failed {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for dma-buf surface attach".to_owned(),
            ));
        }
        if let Err(error) = state.maybe_request_buffer(qh.clone(), render_node_path) {
            return Err(error);
        }
    }

    if state.import_failed {
        return Err(common::TestControl::Fail(
            "server rejected the helper dma-buf buffer".to_owned(),
        ));
    }

    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    thread::sleep(POST_ATTACH_HOLD);

    Ok(DmabufClientSummary {
        globals: state.globals,
        configure_serial: state.configure_serial.ok_or_else(|| {
            common::TestControl::Fail("client never received xdg_surface.configure".to_owned())
        })?,
    })
}

fn dispatch_client_once(
    event_queue: &mut EventQueue<DmabufClientState>,
    state: &mut DmabufClientState,
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

fn create_dmabuf_buffer_params(
    dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    render_node_path: &Path,
    qh: &QueueHandle<DmabufClientState>,
) -> Result<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, common::TestControl> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(render_node_path)
        .map_err(|error| common::TestControl::Skip(format!(
            "failed to open DRM render node {}: {error}",
            render_node_path.display()
        )))?;
    let gbm = GbmDevice::new(file)
        .map_err(|error| common::TestControl::Skip(format!("failed to create GBM device: {error}")))?;
    let mut allocator = GbmAllocator::new(
        gbm,
        GbmBufferFlags::RENDERING | GbmBufferFlags::LINEAR | GbmBufferFlags::WRITE,
    );
    let mut buffer = allocator
        .create_buffer_with_flags(
            TEST_WIDTH,
            TEST_HEIGHT,
            Fourcc::Xrgb8888,
            &[Modifier::Linear, Modifier::Invalid],
            GbmBufferFlags::RENDERING | GbmBufferFlags::LINEAR | GbmBufferFlags::WRITE,
        )
        .map_err(|error| common::TestControl::Skip(format!("failed to allocate GBM buffer: {error}")))?;

    let stride = buffer.pitch() as usize;
    let mut pixels = vec![0_u8; stride * TEST_HEIGHT as usize];
    for row in 0..TEST_HEIGHT as usize {
        let row_start = row * stride;
        for pixel in pixels[row_start..row_start + TEST_WIDTH as usize * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0x44, 0xaa, 0xdd, 0x00]);
        }
    }
    buffer.write(&pixels).map_err(|error| {
        common::TestControl::Fail(format!("failed to write GBM dma-buf backing memory: {error}"))
    })?;

    let dmabuf_handle = buffer
        .export()
        .map_err(|error| common::TestControl::Fail(format!("failed to export GBM buffer as dma-buf: {error}")))?;
    let offsets = dmabuf_handle.offsets().collect::<Vec<_>>();
    let strides = dmabuf_handle.strides().collect::<Vec<_>>();
    let plane_fds = dmabuf_handle
        .handles()
        .map(|fd| fd.try_clone_to_owned())
        .collect::<Result<Vec<OwnedFd>, _>>()
        .map_err(|error| common::TestControl::Fail(format!("failed to duplicate dma-buf fd: {error}")))?;
    let modifier = u64::from(dmabuf_handle.format().modifier);
    let params = dmabuf.create_params(qh, ());
    for (plane_idx, fd) in plane_fds.iter().enumerate() {
        params.add(
            fd.as_fd(),
            plane_idx as u32,
            offsets[plane_idx],
            strides[plane_idx],
            (modifier >> 32) as u32,
            modifier as u32,
        );
    }
    params.create(
        TEST_WIDTH as i32,
        TEST_HEIGHT as i32,
        Fourcc::Xrgb8888 as u32,
        zwp_linux_buffer_params_v1::Flags::empty(),
    );
    Ok(params)
}

fn looks_like_restricted_runtime_error(message: &str) -> bool {
    message.contains("Operation not permitted")
        || message.contains("Permission denied")
        || message.contains("libseat")
        || message.contains("session")
}

impl DmabufClientState {
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

    fn maybe_request_buffer(
        &mut self,
        qh: QueueHandle<Self>,
        render_node_path: &Path,
    ) -> Result<(), common::TestControl> {
        if self.buffer_requested || self.dmabuf.is_none() || self.configure_serial.is_none() {
            return Ok(());
        }
        let Some(dmabuf) = self.dmabuf.as_ref() else {
            return Ok(());
        };

        self.params = Some(create_dmabuf_buffer_params(dmabuf, render_node_path, &qh)?);
        self.buffer_requested = true;
        Ok(())
    }

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
        surface.damage(0, 0, TEST_WIDTH as i32, TEST_HEIGHT as i32);
        surface.commit();
        self.buffer_attached = true;
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for DmabufClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            state.globals.push(interface.clone());

            match interface.as_str() {
                "wl_compositor" => {
                    let compositor =
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ());
                    state.base_surface = Some(compositor.create_surface(qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                "zwp_linux_dmabuf_v1" => {
                    state.dmabuf = Some(
                        registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                            name,
                            version.min(5),
                            qh,
                            (),
                        ),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for DmabufClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for DmabufClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            state.configure_serial = Some(serial);
            xdg_surface.ack_configure(serial);
            state.maybe_attach_buffer();
        }
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for DmabufClientState {
    fn event(
        state: &mut Self,
        _params: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_linux_buffer_params_v1::Event::Created { buffer } => {
                state.buffer = Some(buffer);
                state.params = None;
                state.maybe_attach_buffer();
            }
            zwp_linux_buffer_params_v1::Event::Failed => {
                state.import_failed = true;
                state.params = None;
            }
            _ => {}
        }
    }
}

delegate_noop!(DmabufClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(DmabufClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(DmabufClientState: ignore wl_surface::WlSurface);
delegate_noop!(DmabufClientState: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(DmabufClientState: ignore zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1);
