//! In-process integration test for clipboard IPC query and subscription state.

use std::io::ErrorKind;
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
    BackendInputAction, BackendInputEvent, CompositorClock, KeyboardFocusState, WaylandCommands,
};
use nekoland_ipc::commands::{ClipboardSnapshot, QueryCommand, SelectionOwnerSnapshot};
use nekoland_ipc::{
    IpcCommand, IpcReply, IpcRequest, IpcServerState, IpcSubscription, IpcSubscriptionEvent,
    SubscriptionTopic, send_request_to_path, subscribe_to_path,
};
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
/// Frame on which the synthetic key pump starts forcing focus/input.
const INPUT_PUMP_START_FRAME: u64 = 48;
/// Maximum number of frames the synthetic key pump stays active.
const INPUT_PUMP_FRAMES: u16 = 400;
/// Extra dwell time after setting selection so IPC observers can catch up.
const CLIENT_HOLD_AFTER_SELECTION: Duration = Duration::from_millis(250);

/// Synthetic input pump that helps the clipboard client gain focus and publish selection.
#[derive(Debug, Default, Resource)]
struct ClipboardInputPump {
    /// Remaining frames during which synthetic key input will be injected.
    remaining_frames: u16,
}

/// Summary returned by the helper clipboard client.
#[derive(Debug)]
struct ClipboardClientSummary {
    globals: Vec<String>,
    selection_sent: bool,
}

/// Helper Wayland client state used to publish clipboard selection during the IPC scenario.
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
}

/// Verifies that clipboard state is visible through both IPC subscription events and the IPC query
/// snapshot.
#[test]
fn ipc_reports_clipboard_query_and_subscription_updates() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-ipc-clipboard");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "ipc-clipboard.toml",
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

    let (wayland_socket_path, ipc_socket_path) = {
        let protocol_server_state = common::protocol_server_state(&app);
        let world = app.inner().world();
        let Some(ipc_server_state) = world.get_resource::<IpcServerState>() else {
            panic!("ipc server state should be available immediately after build");
        };

        let wayland_socket_path =
            match (&protocol_server_state.socket_name, &protocol_server_state.startup_error) {
                (Some(socket_name), _) => runtime_dir.path.join(socket_name),
                (None, Some(error)) if error.contains("Operation not permitted") => {
                    eprintln!("skipping IPC clipboard test in restricted environment: {error}");
                    return;
                }
                (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
                (None, None) => panic!("protocol startup produced neither socket nor error"),
            };

        let ipc_socket_path = match (ipc_server_state.listening, &ipc_server_state.startup_error) {
            (true, _) => ipc_server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping IPC clipboard test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        };

        (wayland_socket_path, ipc_socket_path)
    };

    let ipc_thread = thread::spawn(move || {
        let event = wait_for_clipboard_changed(
            &ipc_socket_path,
            IpcSubscription {
                topic: SubscriptionTopic::Clipboard,
                include_payloads: true,
                events: vec!["clipboard_changed".to_owned()],
            },
        )?;
        let snapshot = wait_for_clipboard_query(&ipc_socket_path)?;
        Ok::<_, common::TestControl>((event, snapshot))
    });
    let client_thread = thread::spawn(move || run_clipboard_client(&wayland_socket_path));

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let client_summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping IPC clipboard test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => panic!("clipboard client failed: {reason}"),
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };
    let (event, snapshot) = match ipc_thread.join() {
        Ok(result) => match result {
            Ok(result) => result,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping IPC clipboard test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => panic!("clipboard IPC test failed: {reason}"),
        },
        Err(_) => panic!("ipc thread should exit cleanly"),
    };

    common::assert_globals_present(&client_summary.globals);
    assert!(client_summary.selection_sent, "client should set clipboard selection");
    assert_eq!(event.topic, SubscriptionTopic::Clipboard);
    assert_eq!(event.event, "clipboard_changed");

    let Some(payload) = event.payload else {
        panic!("clipboard_changed should include a payload");
    };
    let Ok(event_snapshot) = serde_json::from_value::<ClipboardSnapshot>(payload) else {
        panic!("clipboard_changed payload should decode");
    };
    assert_eq!(event_snapshot.seat_id, Some(nekoland_ecs::components::SeatId::PRIMARY));
    assert_eq!(event_snapshot.seat_name.as_deref(), Some("seat-0"));
    assert_eq!(event_snapshot.mime_types, vec![TEST_MIME_TYPE.to_owned()]);
    assert_eq!(event_snapshot.owner, Some(SelectionOwnerSnapshot::Compositor));
    assert_eq!(event_snapshot.persisted_mime_types, vec![TEST_MIME_TYPE.to_owned()]);

    assert_eq!(snapshot.seat_id, Some(nekoland_ecs::components::SeatId::PRIMARY));
    assert_eq!(snapshot.seat_name.as_deref(), Some("seat-0"));
    assert_eq!(snapshot.mime_types, vec![TEST_MIME_TYPE.to_owned()]);
    assert_eq!(snapshot.owner, Some(SelectionOwnerSnapshot::Compositor));
    assert_eq!(snapshot.persisted_mime_types, vec![TEST_MIME_TYPE.to_owned()]);
}

/// Injects keyboard focus/input so the clipboard client can publish selection.
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
        device: "ipc-clipboard-test".to_owned(),
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
            return Err(common::TestControl::Fail(
                "timed out waiting to set clipboard selection".to_owned(),
            ));
        }
    }

    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    thread::sleep(CLIENT_HOLD_AFTER_SELECTION);

    Ok(ClipboardClientSummary { globals: state.globals, selection_sent: state.selection_sent })
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

/// Waits for the first `clipboard_changed` event on the clipboard subscription stream.
fn wait_for_clipboard_changed(
    socket_path: &Path,
    subscription: IpcSubscription,
) -> Result<IpcSubscriptionEvent, common::TestControl> {
    let mut stream = subscribe_to_path(socket_path, &subscription).map_err(|error| {
        if ipc_error_is_skippable(&error) {
            common::TestControl::Skip(error.to_string())
        } else {
            common::TestControl::Fail(error.to_string())
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match stream.read_event() {
            Ok(event) => {
                let Some(payload) = event.payload.clone() else {
                    continue;
                };
                // The subscription can emit an initial empty snapshot before the
                // client publishes selection, so only accept the fully populated state.
                let snapshot =
                    serde_json::from_value::<ClipboardSnapshot>(payload).map_err(|error| {
                        common::TestControl::Fail(format!(
                            "clipboard_changed payload should decode: {error}"
                        ))
                    })?;
                if snapshot.owner == Some(SelectionOwnerSnapshot::Compositor)
                    && snapshot.persisted_mime_types == vec![TEST_MIME_TYPE.to_owned()]
                {
                    return Ok(event);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {
                if Instant::now() >= deadline {
                    // Pull the current query snapshot into the timeout message to
                    // make subscription failures easier to diagnose.
                    let snapshot = send_request_to_path(
                        socket_path,
                        &IpcRequest {
                            correlation_id: 99,
                            command: IpcCommand::Query(QueryCommand::GetClipboard),
                        },
                    )
                    .ok()
                    .and_then(|reply| decode_clipboard_reply(reply).ok());
                    return Err(common::TestControl::Fail(format!(
                        "timed out waiting for clipboard_changed (latest_query={snapshot:?})"
                    )));
                }
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => return Err(common::TestControl::Fail(error.to_string())),
        }
    }
}

/// Polls the IPC clipboard query until it returns the expected selection snapshot.
fn wait_for_clipboard_query(socket_path: &Path) -> Result<ClipboardSnapshot, common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        let request = IpcRequest {
            correlation_id: 2,
            command: IpcCommand::Query(QueryCommand::GetClipboard),
        };
        match send_request_to_path(socket_path, &request) {
            Ok(reply) => {
                let snapshot = decode_clipboard_reply(reply)?;
                if snapshot.seat_name.is_some()
                    && snapshot.mime_types == vec![TEST_MIME_TYPE.to_owned()]
                    && snapshot.owner == Some(SelectionOwnerSnapshot::Compositor)
                    && snapshot.persisted_mime_types == vec![TEST_MIME_TYPE.to_owned()]
                {
                    return Ok(snapshot);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => return Err(common::TestControl::Fail(error.to_string())),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for clipboard query to reflect selection".to_owned(),
            ));
        }
    }
}

/// Decodes the clipboard query reply payload into a clipboard snapshot.
fn decode_clipboard_reply(reply: IpcReply) -> Result<ClipboardSnapshot, common::TestControl> {
    if !reply.ok {
        return Err(common::TestControl::Fail(format!(
            "clipboard query failed: {}",
            reply.message
        )));
    }

    let payload = reply.payload.ok_or_else(|| {
        common::TestControl::Fail("clipboard query returned no payload".to_owned())
    })?;
    serde_json::from_value(payload).map_err(|error| {
        common::TestControl::Fail(format!("invalid clipboard query payload: {error}"))
    })
}

/// Identifies retryable transient IPC errors.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Identifies IPC errors that should skip the test in restricted environments.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::WouldBlock | ErrorKind::TimedOut
    ) || error.raw_os_error() == Some(1)
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
            return;
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
            return;
        };
        self.data_device = Some(manager.get_data_device(seat, qh, ()));
    }

    /// Offer one MIME type and claim compositor clipboard ownership for the current serial.
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
