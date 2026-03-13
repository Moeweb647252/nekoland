//! In-process integration test for keyboard repeat info advertised through `wl_keyboard`.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_protocol::ProtocolServerState;
use wayland_client::protocol::{wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

/// Summary returned by the helper client after it observes keyboard repeat info.
#[derive(Debug, Default)]
struct KeyboardRepeatSummary {
    globals: Vec<String>,
    /// Repeat rate advertised through `wl_keyboard.repeat_info`.
    repeat_rate: Option<i32>,
    /// Repeat delay advertised through `wl_keyboard.repeat_info`.
    repeat_delay: Option<i32>,
}

/// Helper Wayland client state used to create one toplevel and wait for `wl_keyboard.repeat_info`.
#[derive(Debug, Default)]
struct KeyboardRepeatClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    /// Last configure serial seen for the helper toplevel.
    configure_serial: Option<u32>,
    /// Cached repeat-info tuple `(rate, delay)` once advertised by the compositor.
    repeat_info: Option<(i32, i32)>,
}

/// Verifies that keyboard repeat parameters exposed to clients match the runtime config.
#[test]
fn keyboard_repeat_info_comes_from_config() {
    let Some(summary) = run_keyboard_repeat_scenario() else {
        return;
    };

    common::assert_globals_present(&summary.globals);
    assert_eq!(summary.repeat_rate, Some(30));
    assert_eq!(summary.repeat_delay, Some(200));
}

/// Runs the helper client scenario and returns the observed repeat parameters.
fn run_keyboard_repeat_scenario() -> Option<KeyboardRepeatSummary> {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-keyboard-repeat-runtime");
    let mut app = build_app(workspace_config_path());
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(48),
    });

    let socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<ProtocolServerState>()
            .expect("protocol server state should be available immediately after build");

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping keyboard repeat test in restricted environment: {error}");
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let client_thread = thread::spawn(move || run_keyboard_repeat_client(&socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let summary = match client_thread.join().expect("client thread should exit cleanly") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping keyboard repeat test in restricted environment: {reason}");
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("keyboard repeat client failed: {reason}"),
    };

    drop(runtime_dir);
    Some(summary)
}

/// Runs the helper client until `wl_keyboard.repeat_info` is received.
fn run_keyboard_repeat_client(
    socket_path: &Path,
) -> Result<KeyboardRepeatSummary, common::TestControl> {
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

    let mut state = KeyboardRepeatClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while !state.is_complete() {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for wl_keyboard.repeat_info".to_owned(),
            ));
        }
    }

    Ok(KeyboardRepeatSummary {
        globals: state.globals,
        repeat_rate: state.repeat_info.map(|(rate, _)| rate),
        repeat_delay: state.repeat_info.map(|(_, delay)| delay),
    })
}

/// Performs one read/dispatch cycle for the helper Wayland client.
fn dispatch_client_once(
    event_queue: &mut EventQueue<KeyboardRepeatClientState>,
    state: &mut KeyboardRepeatClientState,
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

/// Returns the default config path used by this integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

impl Dispatch<wl_registry::WlRegistry, ()> for KeyboardRepeatClientState {
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
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()));
                    state.maybe_create_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ()));
                    state.maybe_create_toplevel(qh);
                }
                "wl_seat" => {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for KeyboardRepeatClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for KeyboardRepeatClientState {
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
            if let Some(surface) = state.base_surface.as_ref() {
                surface.commit();
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for KeyboardRepeatClientState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for KeyboardRepeatClientState {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::RepeatInfo { rate, delay } = event {
            state.repeat_info = Some((rate, delay));
        }
    }
}

delegate_noop!(KeyboardRepeatClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(KeyboardRepeatClientState: ignore wl_surface::WlSurface);
delegate_noop!(KeyboardRepeatClientState: ignore xdg_toplevel::XdgToplevel);

impl KeyboardRepeatClientState {
    /// Creates the helper toplevel once the compositor and XDG shell globals are both bound.
    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let compositor =
            self.compositor.as_ref().expect("compositor presence checked immediately above");
        let wm_base = self.wm_base.as_ref().expect("wm_base presence checked immediately above");

        let base_surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        base_surface.commit();

        self.base_surface = Some(base_surface);
        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    /// Indicates whether the helper client already observed the repeat-info event it needs.
    fn is_complete(&self) -> bool {
        self.configure_serial.is_some() && self.repeat_info.is_some()
    }
}
