use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Query, ResMut, Resource, With};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland::build_app;
use nekoland_backend::traits::SelectedBackend;
use nekoland_core::app::{NekolandApp, RunLoopSettings};
use nekoland_core::schedules::{LayoutSchedule, RenderSchedule};
use nekoland_ecs::components::{WlSurfaceHandle, XdgWindow};
use nekoland_ecs::events::WindowClosed;
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, KeyboardFocusState, PendingBackendInputEvents,
};
use nekoland_protocol::ProtocolServerState;
use nekoland_shell::decorations;
use wayland_client::protocol::{wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

const SUPER_KEYCODE: u32 = 133;
const Q_KEYCODE: u32 = 24;

#[derive(Debug, Default, Resource)]
struct KeybindingInputPump {
    injected: bool,
}

#[derive(Debug, Default, Resource)]
struct ClosedWindowAudit {
    surface_ids: Vec<u64>,
}

#[derive(Debug, Default)]
struct KeybindingClientSummary {
    globals: Vec<String>,
    received_close: bool,
}

#[derive(Debug, Default)]
struct KeybindingClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    configured: bool,
    received_close: bool,
}

#[test]
fn close_window_keybinding_reaches_real_wayland_client() {
    let Some((mut app, summary)) = run_close_window_keybinding_scenario() else {
        return;
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.received_close, "client should receive xdg_toplevel.close");

    let backend_description = app
        .inner()
        .world()
        .get_resource::<SelectedBackend>()
        .map(|backend| backend.description.clone())
        .unwrap_or_default();
    if backend_description.contains("timer fallback") {
        eprintln!(
            "skipping cleanup assertions because the test environment forced {backend_description}"
        );
        return;
    }

    let world = app.inner_mut().world_mut();
    let window_count = world.query::<&XdgWindow>().iter(world).count();
    let focus = world
        .get_resource::<KeyboardFocusState>()
        .expect("keyboard focus state should remain available after the run");
    let audit = world
        .get_resource::<ClosedWindowAudit>()
        .expect("window close audit should be initialized");

    assert_eq!(window_count, 0, "window entity should be cleaned up after the close round-trip");
    assert!(
        focus.focused_surface.is_none(),
        "keyboard focus should clear after the only window is closed"
    );
    assert_eq!(audit.surface_ids.len(), 1, "close keybinding should emit one WindowClosed");
}

fn run_close_window_keybinding_scenario() -> Option<(NekolandApp, KeybindingClientSummary)> {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-keybinding-runtime");
    let config_path = runtime_dir.path.join("keybindings.toml");
    fs::write(&config_path, test_config()).expect("test config should be writable");

    let mut app = build_app(&config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(160),
    });
    app.inner_mut()
        .init_resource::<KeybindingInputPump>()
        .init_resource::<ClosedWindowAudit>()
        .add_systems(
            LayoutSchedule,
            inject_close_keybinding_input.after(decorations::server_decoration_system),
        )
        .add_systems(RenderSchedule, capture_window_closed_messages);

    let socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<ProtocolServerState>()
            .expect("protocol server state should be available immediately after build");

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping keybinding test in restricted environment: {error}");
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let client_thread = thread::spawn(move || run_keybinding_client(&socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let summary = match client_thread.join().expect("client thread should exit cleanly") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping keybinding test in restricted environment: {reason}");
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("keybinding client failed: {reason}"),
    };

    drop(runtime_dir);
    Some((app, summary))
}

fn inject_close_keybinding_input(
    mut pump: ResMut<KeybindingInputPump>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut pending_backend_inputs: ResMut<PendingBackendInputEvents>,
    windows: Query<&WlSurfaceHandle, With<XdgWindow>>,
) {
    if pump.injected {
        return;
    }

    let Some(surface) = windows.iter().next() else {
        return;
    };

    keyboard_focus.focused_surface = Some(surface.id);
    pending_backend_inputs.items.extend([
        BackendInputEvent {
            device: "keybinding-test".to_owned(),
            action: BackendInputAction::Key { keycode: SUPER_KEYCODE, pressed: true },
        },
        BackendInputEvent {
            device: "keybinding-test".to_owned(),
            action: BackendInputAction::Key { keycode: Q_KEYCODE, pressed: true },
        },
    ]);
    pump.injected = true;
}

fn capture_window_closed_messages(
    mut window_closed: MessageReader<WindowClosed>,
    mut audit: ResMut<ClosedWindowAudit>,
) {
    for event in window_closed.read() {
        audit.surface_ids.push(event.surface_id);
    }
}

fn run_keybinding_client(
    socket_path: &Path,
) -> Result<KeybindingClientSummary, common::TestControl> {
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

    let mut state = KeybindingClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut post_close_dispatch = false;

    while !state.received_close || !post_close_dispatch {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if state.received_close {
            if post_close_dispatch {
                break;
            }
            post_close_dispatch = true;
        }
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for close-window keybinding round-trip".to_owned(),
            ));
        }
    }

    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

    Ok(KeybindingClientSummary { globals: state.globals, received_close: state.received_close })
}

fn dispatch_client_once(
    event_queue: &mut EventQueue<KeybindingClientState>,
    state: &mut KeybindingClientState,
) -> Result<(), common::TestControl> {
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
    })?;
    event_queue.flush().map_err(classify_wayland_error)?;

    let Some(read_guard) = event_queue.prepare_read() else {
        return Ok(());
    };

    read_guard.read().map_err(classify_wayland_error)?;
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
    })?;
    Ok(())
}

fn classify_wayland_error(error: wayland_client::backend::WaylandError) -> common::TestControl {
    match error {
        wayland_client::backend::WaylandError::Io(error) => classify_io_error(error),
        other => common::TestControl::Fail(other.to_string()),
    }
}

fn classify_io_error(error: std::io::Error) -> common::TestControl {
    if matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::TimedOut | ErrorKind::WouldBlock
    ) || error.raw_os_error() == Some(1)
    {
        return common::TestControl::Skip(error.to_string());
    }

    common::TestControl::Fail(error.to_string())
}

fn test_config() -> String {
    r##"
default_layout = "tiling"

[theme]
name = "catppuccin-latte"
cursor_theme = "default"
border_color = "#5c7cfa"
background_color = "#f5f7ff"

[input]
focus_follows_mouse = false
repeat_rate = 30

[[outputs]]
name = "Winit-1"
mode = "1280x720@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Q" = "close-window"
"##
    .trim_start()
    .to_owned()
}

impl Dispatch<wl_registry::WlRegistry, ()> for KeybindingClientState {
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
                    state.maybe_create_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.maybe_create_toplevel(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for KeybindingClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for KeybindingClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            state.configured = true;
            xdg_surface.ack_configure(serial);
            if let Some(surface) = state.surface.as_ref() {
                surface.commit();
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for KeybindingClientState {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.received_close = true;
            if let Some(toplevel) = state.toplevel.take() {
                toplevel.destroy();
            }
            if let Some(xdg_surface) = state.xdg_surface.take() {
                xdg_surface.destroy();
            }
            if let Some(surface) = state.surface.take() {
                surface.destroy();
            }
        }
    }
}

impl KeybindingClientState {
    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let compositor =
            self.compositor.as_ref().expect("compositor presence checked immediately above");
        let wm_base = self.wm_base.as_ref().expect("wm_base presence checked immediately above");
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        surface.commit();

        self.surface = Some(surface);
        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }
}

delegate_noop!(KeybindingClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(KeybindingClientState: ignore wl_surface::WlSurface);
