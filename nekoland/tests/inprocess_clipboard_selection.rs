//! In-process integration test for `wl_data_device` clipboard selection reaching ECS state.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::components::{WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompositorClock, KeyboardFocusState,
    WaylandCommands, WaylandFeedback,
};
use nekoland_protocol::resources::ClipboardSelectionState;
use nekoland_shell::decorations;
use wayland_client::protocol::{
    wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source,
    wl_keyboard, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

/// MIME type offered by the helper clipboard client.
const TEST_MIME_TYPE: &str = "text/plain;charset=utf-8";
/// Maximum number of frames the synthetic key pump will stay active.
const INPUT_PUMP_FRAMES: u16 = 400;
/// Frame at which the synthetic key pump begins forcing focus/input.
const INPUT_PUMP_START_FRAME: u64 = 6;
/// Extra dwell time after publishing selection so ECS extraction can catch up.
const CLIENT_HOLD_AFTER_SELECTION: Duration = Duration::from_millis(250);

/// Repeatedly injects keyboard input so the test client can gain focus and set selection.
#[derive(Debug, Default, Resource)]
struct ClipboardInputPump {
    /// Remaining frames during which synthetic key input will be injected.
    remaining_frames: u16,
}

/// Summary returned by the helper clipboard client.
#[derive(Debug, Default)]
struct ClipboardClientSummary {
    globals: Vec<String>,
    selection_sent: bool,
    keyboard_enter_count: usize,
    key_press_count: usize,
    data_device_bound: bool,
}

/// Helper Wayland client state used to create one toplevel, gain keyboard focus, and publish a
/// clipboard selection.
#[derive(Debug, Default)]
struct ClipboardClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    data_device: Option<wl_data_device::WlDataDevice>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    data_source: Option<wl_data_source::WlDataSource>,
    /// Last configure serial seen for the helper toplevel.
    configure_serial: Option<u32>,
    /// Whether the helper toplevel currently owns keyboard focus.
    keyboard_focused: bool,
    /// Whether the helper client already published clipboard selection.
    selection_sent: bool,
    /// How many keyboard-enter events the helper observed.
    keyboard_enter_count: usize,
    /// How many pressed key events the helper observed while running.
    key_press_count: usize,
}

/// Verifies that a clipboard selection set by a real client is mirrored into ECS state.
#[test]
fn clipboard_selection_reaches_ecs_state() {
    let Some((summary, selection_state)) = run_clipboard_selection_scenario() else {
        return;
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.selection_sent, "client should set wl_data_device selection");
    assert!(
        summary.data_device_bound,
        "client should bind wl_data_device before setting selection"
    );
    assert!(summary.keyboard_enter_count >= 1, "client should receive wl_keyboard.enter");
    assert!(summary.key_press_count >= 1, "client should receive at least one wl_keyboard.key");

    let Some(selection) = selection_state.selection else {
        panic!("clipboard selection should be tracked after the client sets it");
    };
    assert_eq!(selection.seat_name, "seat-0");
    assert_eq!(selection.mime_types, vec![TEST_MIME_TYPE.to_owned()]);
}

/// Runs the clipboard selection scenario and returns both the helper-client summary and the ECS
/// clipboard selection state.
fn run_clipboard_selection_scenario() -> Option<(ClipboardClientSummary, ClipboardSelectionState)> {
    let Ok(_env_lock) = common::env_lock().lock() else {
        panic!("environment lock should not be poisoned");
    };
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-clipboard-runtime");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "clipboard-selection.toml",
    );
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(512),
    });
    app.inner_mut()
        .insert_resource(ClipboardInputPump { remaining_frames: INPUT_PUMP_FRAMES })
        .add_systems(
            LayoutSchedule,
            pump_keyboard_selection_input.after(decorations::server_decoration_system),
        );

    let socket_path = match common::protocol_socket_path(&app, &runtime_dir.path) {
        Ok(socket_path) => socket_path,
        Err(error) if error.contains("Operation not permitted") => {
            eprintln!("skipping clipboard selection test in restricted environment: {error}");
            return None;
        }
        Err(error) => panic!("protocol startup failed before run: {error}"),
    };

    let client_thread = thread::spawn(move || run_clipboard_client(&socket_path));
    let Ok(()) = app.run() else {
        panic!("nekoland app should complete the configured frame budget");
    };

    let Some(selection_state) = app
        .inner()
        .world()
        .get_resource::<WaylandFeedback>()
        .map(|feedback| feedback.clipboard_selection.clone())
    else {
        panic!("clipboard selection feedback should be initialized");
    };

    let Ok(client_result) = client_thread.join() else {
        panic!("client thread should exit cleanly");
    };
    let summary = match client_result {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping clipboard selection test in restricted environment: {reason}");
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("clipboard client failed: {reason}"),
    };

    drop(runtime_dir);
    Some((summary, selection_state))
}

/// Injects keyboard focus/input so the clipboard client can publish a selection.
fn pump_keyboard_selection_input(
    clock: Res<CompositorClock>,
    mut pump: ResMut<ClipboardInputPump>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut wayland_commands: ResMut<WaylandCommands>,
    windows: Query<&WlSurfaceHandle, With<XdgWindow>>,
) {
    if pump.remaining_frames == 0 || clock.frame < INPUT_PUMP_START_FRAME {
        return;
    }

    let Some(surface) = windows.iter().next() else {
        return;
    };

    keyboard_focus.focused_surface = Some(surface.id);
    wayland_commands.pending_protocol_input_events.push(BackendInputEvent {
        device: "clipboard-test".to_owned(),
        action: BackendInputAction::Key { keycode: 36, pressed: true },
    });
    pump.remaining_frames = pump.remaining_frames.saturating_sub(1);
}

/// Runs the helper clipboard client until it successfully publishes a selection.
fn run_clipboard_client(socket_path: &Path) -> Result<ClipboardClientSummary, common::TestControl> {
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

    let mut state = ClipboardClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while !state.selection_sent {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting to set clipboard selection (keyboard_enters={}, key_presses={}, data_device_bound={})",
                state.keyboard_enter_count,
                state.key_press_count,
                state.data_device.is_some(),
            )));
        }
    }

    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    thread::sleep(CLIENT_HOLD_AFTER_SELECTION);

    Ok(ClipboardClientSummary {
        globals: state.globals,
        selection_sent: state.selection_sent,
        keyboard_enter_count: state.keyboard_enter_count,
        key_press_count: state.key_press_count,
        data_device_bound: state.data_device.is_some(),
    })
}

/// Performs one read/dispatch cycle for the helper clipboard client.
fn dispatch_client_once(
    event_queue: &mut EventQueue<ClipboardClientState>,
    state: &mut ClipboardClientState,
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

impl Dispatch<wl_registry::WlRegistry, ()> for ClipboardClientState {
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
                "wl_seat" => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ()));
                    state.maybe_bind_data_device(qh);
                }
                "wl_data_device_manager" => {
                    state.data_device_manager =
                        Some(registry.bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(
                            name,
                            3,
                            qh,
                            (),
                        ));
                    state.maybe_bind_data_device(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClipboardClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for ClipboardClientState {
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

impl Dispatch<wl_seat::WlSeat, ()> for ClipboardClientState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
            && state.keyboard.is_none()
        {
            state.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for ClipboardClientState {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                state.keyboard_enter_count += 1;
                state.keyboard_focused = state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id());
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                if state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id())
                {
                    state.keyboard_focused = false;
                }
            }
            wl_keyboard::Event::Key {
                serial,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } if state.keyboard_focused && !state.selection_sent => {
                state.key_press_count += 1;
                state.set_clipboard_selection(qh, serial);
            }
            _ => {}
        }
    }
}

delegate_noop!(ClipboardClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(ClipboardClientState: ignore wl_surface::WlSurface);
delegate_noop!(ClipboardClientState: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(ClipboardClientState: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(ClipboardClientState: ignore wl_data_device::WlDataDevice);
delegate_noop!(ClipboardClientState: ignore wl_data_source::WlDataSource);
delegate_noop!(ClipboardClientState: ignore wl_data_offer::WlDataOffer);

impl ClipboardClientState {
    /// Create the helper toplevel once both compositor globals are available.
    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let (Some(compositor), Some(wm_base)) = (self.compositor.as_ref(), self.wm_base.as_ref())
        else {
            panic!("compositor and wm_base presence checked immediately above");
        };

        let base_surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        base_surface.commit();

        self.base_surface = Some(base_surface);
        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    /// Bind the seat-scoped data device once both the seat and manager are known.
    fn maybe_bind_data_device(&mut self, qh: &QueueHandle<Self>) {
        if self.data_device.is_some() || self.data_device_manager.is_none() || self.seat.is_none() {
            return;
        }

        let (Some(manager), Some(seat)) = (self.data_device_manager.as_ref(), self.seat.as_ref())
        else {
            panic!("data-device manager and seat presence checked immediately above");
        };
        self.data_device = Some(manager.get_data_device(seat, qh, ()));
    }

    /// Offer one MIME type and claim clipboard ownership for the current serial.
    fn set_clipboard_selection(&mut self, qh: &QueueHandle<Self>, serial: u32) {
        let Some(manager) = self.data_device_manager.as_ref() else {
            return;
        };
        let Some(data_device) = self.data_device.as_ref() else {
            return;
        };

        let source = manager.create_data_source(qh, ());
        source.offer(TEST_MIME_TYPE.to_owned());
        data_device.set_selection(Some(&source), serial);
        self.data_source = Some(source);
        self.selection_sent = true;
    }
}
