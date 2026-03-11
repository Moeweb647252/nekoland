use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    PendingProtocolInputEvents, PrimarySelectionState, SelectionOwner,
};
use nekoland_protocol::ProtocolServerState;
use nekoland_shell::decorations;
use wayland_client::protocol::{wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_surface};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1, zwp_primary_selection_source_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

const TEST_SELECTION_BYTES: &[u8] = b"nekoland primary selection roundtrip";
const TEST_MIME_TYPE: &str = "text/plain;charset=utf-8";
const INPUT_PUMP_START_FRAME: u64 = 6;
const MAX_TEST_FRAMES: u64 = 4096;

#[derive(Debug, Resource)]
struct PrimarySelectionTransferPump {
    source_selection_sent: Arc<AtomicBool>,
}

#[derive(Debug)]
struct SourceClientSummary {
    globals: Vec<String>,
    selection_sent: bool,
    send_requests: usize,
}

#[derive(Debug)]
struct TargetClientSummary {
    globals: Vec<String>,
    received_payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct SourceClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    primary_selection_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    primary_selection_device: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    primary_selection_source: Option<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1>,
    configure_serial: Option<u32>,
    keyboard_focused: bool,
    selection_sent: bool,
    send_requests: usize,
}

#[derive(Debug, Default)]
struct TargetClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    primary_selection_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    primary_selection_device: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    configure_serial: Option<u32>,
    selection_offer: Option<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1>,
    pending_read: Option<std::os::unix::net::UnixStream>,
    receive_requested: bool,
    received_payload: Vec<u8>,
}

#[test]
fn primary_selection_roundtrips_between_two_real_clients() {
    let Some((source, target, selection_state)) = run_primary_selection_transfer_scenario() else {
        return;
    };

    common::assert_globals_present(&source.globals);
    common::assert_globals_present(&target.globals);
    assert!(source.selection_sent, "source client should set primary selection");
    assert!(source.send_requests >= 1, "source client should serve at least one primary send");
    assert_eq!(target.received_payload, TEST_SELECTION_BYTES);

    let selection = selection_state
        .selection
        .expect("primary selection should remain tracked after the transfer");
    assert_eq!(selection.seat_name, "seat-0");
    assert_eq!(selection.mime_types, vec![TEST_MIME_TYPE.to_owned()]);
    assert_eq!(selection.owner, SelectionOwner::Compositor);
    assert_eq!(selection.persisted_mime_types, vec![TEST_MIME_TYPE.to_owned()]);
}

#[test]
fn primary_selection_persists_after_source_client_exits() {
    let Some((source, target, selection_state)) = run_primary_selection_persistence_scenario()
    else {
        return;
    };

    common::assert_globals_present(&source.globals);
    common::assert_globals_present(&target.globals);
    assert!(source.selection_sent, "source client should set primary selection");
    assert!(
        source.send_requests >= 1,
        "source client should serve at least one compositor capture request"
    );
    assert_eq!(target.received_payload, TEST_SELECTION_BYTES);

    let selection = selection_state
        .selection
        .expect("primary selection should remain tracked after the source exits");
    assert_eq!(selection.seat_name, "seat-0");
    assert_eq!(selection.mime_types, vec![TEST_MIME_TYPE.to_owned()]);
    assert_eq!(selection.owner, SelectionOwner::Compositor);
    assert_eq!(selection.persisted_mime_types, vec![TEST_MIME_TYPE.to_owned()]);
}

fn run_primary_selection_transfer_scenario()
-> Option<(SourceClientSummary, TargetClientSummary, PrimarySelectionState)> {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-primary-selection-transfer-runtime");
    let source_selection_sent = Arc::new(AtomicBool::new(false));
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "primary-selection-transfer.toml",
    );
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(MAX_TEST_FRAMES),
    });
    app.inner_mut()
        .insert_resource(PrimarySelectionTransferPump {
            source_selection_sent: source_selection_sent.clone(),
        })
        .add_systems(
            LayoutSchedule,
            pump_primary_selection_transfer_input.after(decorations::server_decoration_system),
        );

    let socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<ProtocolServerState>()
            .expect("protocol server state should be available immediately after build");

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!(
                    "skipping primary selection transfer test in restricted environment: {error}"
                );
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let source_socket_path = socket_path.clone();
    let source_flag = source_selection_sent.clone();
    let source_thread = thread::spawn(move || run_source_client(&source_socket_path, source_flag));
    let target_flag = source_selection_sent.clone();
    let target_thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !target_flag.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for source client to publish primary selection".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }

        run_target_client(&socket_path)
    });

    app.run().expect("nekoland app should complete the configured frame budget");

    let selection_state = app
        .inner()
        .world()
        .get_resource::<PrimarySelectionState>()
        .cloned()
        .expect("primary selection resource should be initialized");

    let target_summary = match target_thread.join().expect("target client thread should join") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!(
                "skipping primary selection transfer test in restricted environment: {reason}"
            );
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("target client failed: {reason}"),
    };
    let source_summary = match source_thread.join().expect("source client thread should join") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!(
                "skipping primary selection transfer test in restricted environment: {reason}"
            );
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("source client failed: {reason}"),
    };

    drop(runtime_dir);
    Some((source_summary, target_summary, selection_state))
}

fn run_primary_selection_persistence_scenario()
-> Option<(SourceClientSummary, TargetClientSummary, PrimarySelectionState)> {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir =
        common::RuntimeDirGuard::new("nekoland-primary-selection-persistence-runtime");
    let source_selection_sent = Arc::new(AtomicBool::new(false));
    let source_exited = Arc::new(AtomicBool::new(false));
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "primary-selection-persistence.toml",
    );
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(MAX_TEST_FRAMES),
    });
    app.inner_mut()
        .insert_resource(PrimarySelectionTransferPump {
            source_selection_sent: source_selection_sent.clone(),
        })
        .add_systems(
            LayoutSchedule,
            pump_primary_selection_transfer_input.after(decorations::server_decoration_system),
        );

    let socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<ProtocolServerState>()
            .expect("protocol server state should be available immediately after build");

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!(
                    "skipping primary selection persistence test in restricted environment: {error}"
                );
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let source_socket_path = socket_path.clone();
    let source_flag = source_selection_sent.clone();
    let source_exited_flag = source_exited.clone();
    let source_thread = thread::spawn(move || {
        let result = run_source_client(&source_socket_path, source_flag);
        source_exited_flag.store(true, Ordering::SeqCst);
        result
    });
    let target_thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !source_exited.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for source client to exit after publishing primary selection"
                        .to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }

        run_target_client(&socket_path)
    });

    app.run().expect("nekoland app should complete the configured frame budget");

    let selection_state = app
        .inner()
        .world()
        .get_resource::<PrimarySelectionState>()
        .cloned()
        .expect("primary selection resource should be initialized");

    let target_summary = match target_thread.join().expect("target client thread should join") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!(
                "skipping primary selection persistence test in restricted environment: {reason}"
            );
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("target client failed: {reason}"),
    };
    let source_summary = match source_thread.join().expect("source client thread should join") {
        Ok(summary) => summary,
        Err(common::TestControl::Skip(reason)) => {
            eprintln!(
                "skipping primary selection persistence test in restricted environment: {reason}"
            );
            return None;
        }
        Err(common::TestControl::Fail(reason)) => panic!("source client failed: {reason}"),
    };

    drop(runtime_dir);
    Some((source_summary, target_summary, selection_state))
}

fn pump_primary_selection_transfer_input(
    clock: Res<CompositorClock>,
    pump: Res<PrimarySelectionTransferPump>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut pending_protocol_inputs: ResMut<PendingProtocolInputEvents>,
    windows: Query<&WlSurfaceHandle, With<XdgWindow>>,
) {
    if clock.frame < INPUT_PUMP_START_FRAME {
        return;
    }

    let mut surface_ids = windows.iter().map(|surface| surface.id).collect::<Vec<_>>();
    surface_ids.sort_unstable();
    if surface_ids.is_empty() {
        return;
    }

    if pump.source_selection_sent.load(Ordering::SeqCst) {
        keyboard_focus.focused_surface =
            Some(*surface_ids.get(1).unwrap_or_else(|| surface_ids.first().expect("non-empty")));
    } else {
        keyboard_focus.focused_surface = Some(surface_ids[0]);
        pending_protocol_inputs.items.push(BackendInputEvent {
            device: "primary-selection-transfer".to_owned(),
            action: BackendInputAction::Key { keycode: 36, pressed: true },
        });
    }
}

fn run_source_client(
    socket_path: &Path,
    selection_flag: Arc<AtomicBool>,
) -> Result<SourceClientSummary, common::TestControl> {
    let stream =
        std::os::unix::net::UnixStream::connect(socket_path).map_err(classify_client_io)?;
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

    let mut state = SourceClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while state.send_requests == 0 {
        dispatch_source_client_once(&mut event_queue, &mut state)?;
        if state.selection_sent {
            selection_flag.store(true, Ordering::SeqCst);
        }
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for source primary send request (selection_sent={})",
                state.selection_sent,
            )));
        }
    }

    Ok(SourceClientSummary {
        globals: state.globals,
        selection_sent: state.selection_sent,
        send_requests: state.send_requests,
    })
}

fn dispatch_source_client_once(
    event_queue: &mut EventQueue<SourceClientState>,
    state: &mut SourceClientState,
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

fn run_target_client(socket_path: &Path) -> Result<TargetClientSummary, common::TestControl> {
    let stream =
        std::os::unix::net::UnixStream::connect(socket_path).map_err(classify_client_io)?;
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

    let mut state = TargetClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while state.received_payload != TEST_SELECTION_BYTES {
        dispatch_target_client_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for primary selection payload (receive_requested={}, offer_present={})",
                state.receive_requested,
                state.selection_offer.is_some(),
            )));
        }
    }

    Ok(TargetClientSummary { globals: state.globals, received_payload: state.received_payload })
}

fn dispatch_target_client_once(
    event_queue: &mut EventQueue<TargetClientState>,
    state: &mut TargetClientState,
) -> Result<(), common::TestControl> {
    state.try_read_received_payload()?;
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
    })?;
    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

    let Some(read_guard) = event_queue.prepare_read() else {
        state.try_read_received_payload()?;
        return Ok(());
    };

    read_guard.read().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
    })?;
    state.try_read_received_payload()?;
    Ok(())
}

fn classify_client_io(error: std::io::Error) -> common::TestControl {
    common::TestControl::Fail(error.to_string())
}

impl Dispatch<wl_registry::WlRegistry, ()> for SourceClientState {
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
                    state.maybe_bind_primary_selection_device(qh);
                }
                "zwp_primary_selection_device_manager_v1" => {
                    state.primary_selection_manager = Some(registry.bind::<
                        zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
                        _,
                        _,
                    >(name, 1, qh, ()));
                    state.maybe_bind_primary_selection_device(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for TargetClientState {
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
                    state.maybe_bind_primary_selection_device(qh);
                }
                "zwp_primary_selection_device_manager_v1" => {
                    state.primary_selection_manager = Some(registry.bind::<
                        zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
                        _,
                        _,
                    >(name, 1, qh, ()));
                    state.maybe_bind_primary_selection_device(qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for SourceClientState {
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TargetClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for SourceClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for TargetClientState {
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

impl Dispatch<wl_seat::WlSeat, ()> for SourceClientState {
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

impl Dispatch<wl_seat::WlSeat, ()> for TargetClientState {
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

impl Dispatch<wl_keyboard::WlKeyboard, ()> for SourceClientState {
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
                state.set_selection(qh, serial);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TargetClientState {
    fn event(
        _state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        _event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()>
    for SourceClientState
{
    event_created_child!(SourceClientState, zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, [
        0 => (zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ())
    ]);

    fn event(
        _state: &mut Self,
        _device: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        _event: zwp_primary_selection_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()>
    for TargetClientState
{
    event_created_child!(TargetClientState, zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, [
        0 => (zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ())
    ]);

    fn event(
        state: &mut Self,
        _device: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_device_v1::Event::Selection { id: Some(offer) } => {
                state.selection_offer = Some(offer);
                let _ = state.maybe_request_receive();
            }
            zwp_primary_selection_device_v1::Event::Selection { id: None } => {
                state.selection_offer = None;
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()>
    for SourceClientState
{
    fn event(
        state: &mut Self,
        source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_source_v1::Event::Send { mime_type, fd }
                if mime_type == TEST_MIME_TYPE =>
            {
                let mut file = std::fs::File::from(fd);
                file.write_all(TEST_SELECTION_BYTES)
                    .expect("source client should write primary selection payload");
                state.send_requests = state.send_requests.saturating_add(1);
            }
            zwp_primary_selection_source_v1::Event::Cancelled
                if state.primary_selection_source.as_ref() == Some(source) =>
            {
                state.primary_selection_source = None;
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()>
    for TargetClientState
{
    fn event(
        _state: &mut Self,
        _source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        _event: zwp_primary_selection_source_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()>
    for SourceClientState
{
    fn event(
        _state: &mut Self,
        _offer: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        _event: zwp_primary_selection_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()>
    for TargetClientState
{
    fn event(
        state: &mut Self,
        offer: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if matches!(
            event,
            zwp_primary_selection_offer_v1::Event::Offer { mime_type }
                if mime_type == TEST_MIME_TYPE
        ) && state.selection_offer.as_ref().is_some_and(|current| current.id() == offer.id())
        {
            let _ = state.maybe_request_receive();
        }
    }
}

delegate_noop!(SourceClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(SourceClientState: ignore wl_surface::WlSurface);
delegate_noop!(SourceClientState: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(SourceClientState: ignore zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1);

delegate_noop!(TargetClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(TargetClientState: ignore wl_surface::WlSurface);
delegate_noop!(TargetClientState: ignore xdg_toplevel::XdgToplevel);
delegate_noop!(TargetClientState: ignore zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1);

impl SourceClientState {
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

    fn maybe_bind_primary_selection_device(&mut self, qh: &QueueHandle<Self>) {
        if self.primary_selection_device.is_some()
            || self.primary_selection_manager.is_none()
            || self.seat.is_none()
        {
            return;
        }

        let manager = self
            .primary_selection_manager
            .as_ref()
            .expect("primary-selection manager presence checked immediately above");
        let seat = self.seat.as_ref().expect("seat presence checked immediately above");
        self.primary_selection_device = Some(manager.get_device(seat, qh, ()));
    }

    fn set_selection(&mut self, qh: &QueueHandle<Self>, serial: u32) {
        let Some(manager) = self.primary_selection_manager.as_ref() else {
            return;
        };
        let Some(device) = self.primary_selection_device.as_ref() else {
            return;
        };

        let source = manager.create_source(qh, ());
        source.offer(TEST_MIME_TYPE.to_owned());
        device.set_selection(Some(&source), serial);
        self.primary_selection_source = Some(source);
        self.selection_sent = true;
    }
}

impl TargetClientState {
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

    fn maybe_bind_primary_selection_device(&mut self, qh: &QueueHandle<Self>) {
        if self.primary_selection_device.is_some()
            || self.primary_selection_manager.is_none()
            || self.seat.is_none()
        {
            return;
        }

        let manager = self
            .primary_selection_manager
            .as_ref()
            .expect("primary-selection manager presence checked immediately above");
        let seat = self.seat.as_ref().expect("seat presence checked immediately above");
        self.primary_selection_device = Some(manager.get_device(seat, qh, ()));
    }

    fn maybe_request_receive(&mut self) -> Result<(), common::TestControl> {
        if self.receive_requested {
            return Ok(());
        }

        let Some(offer) = self.selection_offer.as_ref() else {
            return Ok(());
        };

        let (reader, writer) = std::os::unix::net::UnixStream::pair()
            .map_err(|error| common::TestControl::Fail(error.to_string()))?;
        writer
            .shutdown(std::net::Shutdown::Read)
            .map_err(|error| common::TestControl::Fail(error.to_string()))?;
        reader
            .set_nonblocking(true)
            .map_err(|error| common::TestControl::Fail(error.to_string()))?;

        offer.receive(TEST_MIME_TYPE.to_owned(), writer.as_fd());
        self.pending_read = Some(reader);
        self.receive_requested = true;
        Ok(())
    }

    fn try_read_received_payload(&mut self) -> Result<(), common::TestControl> {
        let Some(stream) = self.pending_read.as_mut() else {
            return Ok(());
        };

        let mut buffer = [0_u8; 256];
        match stream.read(&mut buffer) {
            Ok(0) => Ok(()),
            Ok(read) => {
                self.received_payload.extend_from_slice(&buffer[..read]);
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(common::TestControl::Fail(error.to_string())),
        }
    }
}
