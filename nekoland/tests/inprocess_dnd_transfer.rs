//! In-process integration test for drag-and-drop data transfer between two real clients.

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::SystemParam;
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::components::{SurfaceGeometry, WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompositorClock, DragAndDropState,
    GlobalPointerPosition, KeyboardFocusState, PendingProtocolInputEvents, PendingWindowControls,
};
use nekoland_ecs::selectors::SurfaceId;
use nekoland_protocol::ProtocolServerState;
use nekoland_shell::decorations;
use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

/// Pointer button code used to start and finish the drag.
const TEST_BUTTON_CODE: u32 = 0x110;
/// MIME type offered for the drag-and-drop transfer.
const TEST_MIME_TYPE: &str = "text/plain;charset=utf-8";
/// Payload transferred from the source client to the target client.
const TEST_DND_BYTES: &[u8] = b"nekoland dnd roundtrip";
/// Width of the helper clients' SHM buffers.
const TEST_BUFFER_WIDTH: u32 = 48;
/// Height of the helper clients' SHM buffers.
const TEST_BUFFER_HEIGHT: u32 = 48;
/// Frame at which the synthetic DnD pump begins driving pointer state.
const INPUT_PUMP_START_FRAME: u64 = 8;
/// Generous frame budget for the two-client DnD scenario.
const MAX_TEST_FRAMES: u64 = 1024;
const SOURCE_WINDOW_X: isize = 120;
const SOURCE_WINDOW_Y: isize = 120;
const TARGET_WINDOW_X: isize = 420;
const TARGET_WINDOW_Y: isize = 120;
const DND_WINDOW_WIDTH: u32 = 240;
const DND_WINDOW_HEIGHT: u32 = 180;

/// High-level state machine for the synthetic input pump that drives the DnD scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DndPumpPhase {
    WaitForWindows,
    MoveToSource,
    WaitForSourceFocus,
    PressSource,
    WaitForDragStart,
    MoveToTarget,
    WaitForTargetOffer,
    ReleaseOnTarget,
    Done,
}

/// Test pump that drives the pointer/button choreography needed for the drag-and-drop scenario.
#[derive(Debug, Resource)]
struct DndTransferPump {
    /// Flag set once the source toplevel exists and has committed a buffer.
    source_window_ready: Arc<AtomicBool>,
    /// Flag set once the source client observed pointer entry.
    source_pointer_ready: Arc<AtomicBool>,
    /// Flag set once the source client called `start_drag`.
    source_drag_started: Arc<AtomicBool>,
    /// Flag set once the target client exists and has committed a buffer.
    target_ready: Arc<AtomicBool>,
    /// Flag set once the target observed a matching DnD offer.
    target_offer_ready: Arc<AtomicBool>,
    /// Whether the helper windows have been arranged into deterministic floating slots.
    windows_arranged: bool,
    /// Current phase of the synthetic pointer choreography.
    phase: DndPumpPhase,
    /// Retry counter while waiting for source focus/pointer entry.
    source_focus_attempts: u8,
    /// Retry counter while waiting for the target offer negotiation to settle.
    target_offer_attempts: u8,
}

#[derive(SystemParam)]
struct DndTransferPumpParams<'w, 's> {
    clock: Res<'w, CompositorClock>,
    dnd_state: Res<'w, DragAndDropState>,
    pump: ResMut<'w, DndTransferPump>,
    keyboard_focus: ResMut<'w, KeyboardFocusState>,
    pointer: ResMut<'w, GlobalPointerPosition>,
    pending_protocol_inputs: ResMut<'w, PendingProtocolInputEvents>,
    pending_window_controls: ResMut<'w, PendingWindowControls>,
    windows: Query<
        'w,
        's,
        (&'static WlSurfaceHandle, &'static mut SurfaceGeometry, &'static XdgWindow),
        With<XdgWindow>,
    >,
}

/// Summary returned by the source DnD client.
#[derive(Debug)]
struct SourceClientSummary {
    globals: Vec<String>,
    drag_started: bool,
    send_requests: usize,
}

/// Summary returned by the target DnD client.
#[derive(Debug)]
struct TargetClientSummary {
    globals: Vec<String>,
    received_payload: Vec<u8>,
}

/// Source DnD client state that initiates the drag and serves offered data.
#[derive(Debug, Default)]
struct SourceClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    data_device: Option<wl_data_device::WlDataDevice>,
    shm: Option<wl_shm::WlShm>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    data_source: Option<wl_data_source::WlDataSource>,
    _pool: Option<wl_shm_pool::WlShmPool>,
    _buffer: Option<wl_buffer::WlBuffer>,
    _backing_file: Option<std::fs::File>,
    /// Whether the helper toplevel received its initial configure.
    configured: bool,
    /// Whether the helper attached a real buffer and became mappable.
    buffer_attached: bool,
    /// Last compositor-suggested width for the toplevel, if non-zero.
    configured_width: Option<u32>,
    /// Last compositor-suggested height for the toplevel, if non-zero.
    configured_height: Option<u32>,
    /// Whether the pointer is currently inside the source surface.
    pointer_inside: bool,
    /// Whether the source already started the drag operation.
    drag_started: bool,
    /// Number of `wl_data_source.send` requests served so far.
    send_requests: usize,
}

/// Target DnD client state that accepts the drop and reads the offered data.
#[derive(Debug, Default)]
struct TargetClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    seat: Option<wl_seat::WlSeat>,
    data_device: Option<wl_data_device::WlDataDevice>,
    shm: Option<wl_shm::WlShm>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    _pool: Option<wl_shm_pool::WlShmPool>,
    _buffer: Option<wl_buffer::WlBuffer>,
    _backing_file: Option<std::fs::File>,
    /// Whether the helper toplevel received its initial configure.
    configured: bool,
    /// Whether the helper attached a real buffer and became mappable.
    buffer_attached: bool,
    /// Last compositor-suggested width for the toplevel, if non-zero.
    configured_width: Option<u32>,
    /// Last compositor-suggested height for the toplevel, if non-zero.
    configured_height: Option<u32>,
    /// Current drag offer announced by the compositor, if any.
    drag_offer: Option<wl_data_offer::WlDataOffer>,
    /// Serial from the most recent `wl_data_device.enter`.
    enter_serial: Option<u32>,
    /// Whether the target observed the MIME type this test expects.
    offered_test_mime: bool,
    /// Whether the target already requested `offer.receive`.
    receive_requested: bool,
    /// Pipe reader used to collect bytes from the accepted drop.
    pending_read: Option<std::os::unix::net::UnixStream>,
    /// Bytes collected from the drag source.
    received_payload: Vec<u8>,
    /// Number of `wl_data_device.enter` events observed.
    enter_count: usize,
    /// Number of `wl_data_offer.offer` events observed.
    offer_count: usize,
    /// Number of `wl_data_device.drop` events observed.
    drop_count: usize,
    /// Number of `wl_data_device.leave` events observed.
    leave_count: usize,
}

/// Verifies that a drag-and-drop transfer round-trips through the compositor and is reflected in
/// ECS drag-and-drop state.
#[test]
fn drag_and_drop_roundtrips_between_two_real_clients() {
    let Some((source, target, dnd_state)) = run_dnd_transfer_scenario() else {
        return;
    };

    common::assert_globals_present(&source.globals);
    common::assert_globals_present(&target.globals);
    assert!(source.drag_started, "source client should start drag-and-drop");
    assert!(source.send_requests >= 1, "source client should serve at least one DnD send request");
    assert_eq!(target.received_payload, TEST_DND_BYTES);

    assert!(
        dnd_state.active_session.is_none(),
        "drag-and-drop session should be inactive after the drop completes"
    );
    let Some(drop) = dnd_state.last_drop else {
        panic!("drag-and-drop state should record the completed drop");
    };
    assert_eq!(drop.seat_name, "seat-0");
    assert!(drop.validated, "drop should be negotiated and accepted");
    assert_eq!(drop.mime_types, vec![TEST_MIME_TYPE.to_owned()]);
    assert!(drop.source_surface_id.is_some(), "drop should record the source surface");
    assert!(drop.target_surface_id.is_some(), "drop should record the target surface");
    assert_ne!(
        drop.source_surface_id, drop.target_surface_id,
        "source and target surfaces should differ during inter-client DnD"
    );
}

/// Runs the full drag-and-drop transfer scenario between a source and a target client.
fn run_dnd_transfer_scenario()
-> Option<(SourceClientSummary, TargetClientSummary, DragAndDropState)> {
    let Ok(_env_lock) = common::env_lock().lock() else {
        panic!("environment lock should not be poisoned");
    };
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-dnd-transfer-runtime");
    let source_window_ready = Arc::new(AtomicBool::new(false));
    let source_pointer_ready = Arc::new(AtomicBool::new(false));
    let source_drag_started = Arc::new(AtomicBool::new(false));
    let target_client_ready = Arc::new(AtomicBool::new(false));
    let target_offer_ready = Arc::new(AtomicBool::new(false));

    let config_path =
        common::write_default_config_with_xwayland_disabled(&runtime_dir.path, "dnd-transfer.toml");
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(MAX_TEST_FRAMES),
    });
    app.inner_mut()
        .insert_resource(DndTransferPump {
            source_window_ready: source_window_ready.clone(),
            source_pointer_ready: source_pointer_ready.clone(),
            source_drag_started: source_drag_started.clone(),
            target_ready: target_client_ready.clone(),
            target_offer_ready: target_offer_ready.clone(),
            windows_arranged: false,
            phase: DndPumpPhase::WaitForWindows,
            source_focus_attempts: 0,
            target_offer_attempts: 0,
        })
        .add_systems(
            LayoutSchedule,
            pump_dnd_transfer_input.after(decorations::server_decoration_system),
        );

    let socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<ProtocolServerState>() else {
            panic!("protocol server state should be available immediately after build");
        };

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping DnD transfer test in restricted environment: {error}");
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let source_socket_path = socket_path.clone();
    let source_ready_flag = source_window_ready.clone();
    let source_pointer_flag = source_pointer_ready.clone();
    let source_drag_flag = source_drag_started.clone();
    let source_thread = thread::spawn(move || {
        run_source_client(
            &source_socket_path,
            source_ready_flag,
            source_pointer_flag,
            source_drag_flag,
        )
    });

    let target_socket_path = socket_path.clone();
    let target_wait_flag = source_window_ready.clone();
    let target_ready_flag = target_client_ready.clone();
    let target_offer_flag = target_offer_ready.clone();
    let target_thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !target_wait_flag.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for source client window to become ready".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }

        run_target_client(&target_socket_path, target_ready_flag, target_offer_flag)
    });

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let Some(dnd_state) = app.inner().world().get_resource::<DragAndDropState>().cloned() else {
        panic!("drag-and-drop state resource should be initialized");
    };

    let source_result = match source_thread.join() {
        Ok(result) => result,
        Err(_) => panic!("source client thread should join"),
    };
    let target_result = match target_thread.join() {
        Ok(result) => result,
        Err(_) => panic!("target client thread should join"),
    };

    if let Err(common::TestControl::Skip(reason)) = &source_result {
        eprintln!("skipping DnD transfer test in restricted environment: {reason}");
        return None;
    }
    if let Err(common::TestControl::Skip(reason)) = &target_result {
        eprintln!("skipping DnD transfer test in restricted environment: {reason}");
        return None;
    }

    let source_summary = match source_result {
        Ok(summary) => summary,
        Err(common::TestControl::Fail(reason)) => panic!(
            "source client failed: {reason}; target_result={target_result:?}; dnd_state={dnd_state:?}"
        ),
        Err(common::TestControl::Skip(_)) => unreachable!("skip handled above"),
    };
    let target_summary = match target_result {
        Ok(summary) => summary,
        Err(common::TestControl::Fail(reason)) => panic!(
            "target client failed: {reason}; source_summary={source_summary:?}; dnd_state={dnd_state:?}"
        ),
        Err(common::TestControl::Skip(_)) => unreachable!("skip handled above"),
    };

    drop(runtime_dir);
    Some((source_summary, target_summary, dnd_state))
}

/// Drives the synthetic pointer/button choreography that causes the two test clients to perform
/// a drag-and-drop transfer.
fn pump_dnd_transfer_input(transfer: DndTransferPumpParams<'_, '_>) {
    let DndTransferPumpParams {
        clock,
        dnd_state,
        mut pump,
        mut keyboard_focus,
        mut pointer,
        mut pending_protocol_inputs,
        mut pending_window_controls,
        mut windows,
    } = transfer;

    if clock.frame < INPUT_PUMP_START_FRAME || pump.phase == DndPumpPhase::Done {
        return;
    }

    let mut known_windows = windows
        .iter_mut()
        .map(|(surface, geometry, window)| (surface.id, geometry.clone(), window.title.clone()))
        .collect::<Vec<_>>();
    known_windows.sort_by_key(|(surface_id, _, _)| *surface_id);

    // The helper clients label themselves via toplevel titles so the pump can
    // keep source and target roles straight even if entity ordering changes.
    let source_window = known_windows
        .iter()
        .find(|(_, _, title)| title == "dnd-source")
        .cloned()
        .or_else(|| known_windows.first().cloned());
    let target_window =
        known_windows.iter().find(|(_, _, title)| title == "dnd-target").cloned().or_else(|| {
            (known_windows.len() >= 2).then(|| known_windows[known_windows.len() - 1].clone())
        });

    let (
        Some((source_surface_id, source_geometry, _)),
        Some((target_surface_id, target_geometry, _)),
    ) = (source_window, target_window)
    else {
        return;
    };

    let source_position = pointer_in_geometry(&source_geometry);
    let target_position = pointer_in_geometry(&target_geometry);

    match pump.phase {
        DndPumpPhase::WaitForWindows => {
            if !pump.windows_arranged {
                pending_window_controls
                    .surface(SurfaceId(source_surface_id))
                    .move_to(SOURCE_WINDOW_X, SOURCE_WINDOW_Y)
                    .resize_to(DND_WINDOW_WIDTH, DND_WINDOW_HEIGHT)
                    .focus();
                pending_window_controls
                    .surface(SurfaceId(target_surface_id))
                    .move_to(TARGET_WINDOW_X, TARGET_WINDOW_Y)
                    .resize_to(DND_WINDOW_WIDTH, DND_WINDOW_HEIGHT);
                pump.windows_arranged = geometry_matches(
                    &source_geometry,
                    SOURCE_WINDOW_X as i32,
                    SOURCE_WINDOW_Y as i32,
                    DND_WINDOW_WIDTH,
                    DND_WINDOW_HEIGHT,
                ) && geometry_matches(
                    &target_geometry,
                    TARGET_WINDOW_X as i32,
                    TARGET_WINDOW_Y as i32,
                    DND_WINDOW_WIDTH,
                    DND_WINDOW_HEIGHT,
                );
                if !pump.windows_arranged {
                    return;
                }
            }
            if !pump.source_window_ready.load(Ordering::SeqCst) {
                return;
            }
            pump.phase = DndPumpPhase::MoveToSource;
        }
        DndPumpPhase::MoveToSource => {
            keyboard_focus.focused_surface = Some(source_surface_id);
            apply_pointer_motion(
                &mut pointer,
                &mut pending_protocol_inputs,
                source_position.0,
                source_position.1,
            );
            pump.phase = DndPumpPhase::WaitForSourceFocus;
        }
        DndPumpPhase::WaitForSourceFocus => {
            if pump.source_pointer_ready.load(Ordering::SeqCst) {
                pump.phase = DndPumpPhase::PressSource;
            } else {
                keyboard_focus.focused_surface = Some(source_surface_id);
                apply_pointer_motion(
                    &mut pointer,
                    &mut pending_protocol_inputs,
                    source_position.0,
                    source_position.1,
                );
                pump.source_focus_attempts = pump.source_focus_attempts.saturating_add(1);
            }
        }
        DndPumpPhase::PressSource => {
            pending_protocol_inputs.push(BackendInputEvent {
                device: "dnd-test".to_owned(),
                action: BackendInputAction::PointerButton {
                    button_code: TEST_BUTTON_CODE,
                    pressed: true,
                },
            });
            pump.phase = DndPumpPhase::WaitForDragStart;
        }
        DndPumpPhase::WaitForDragStart => {
            if pump.source_drag_started.load(Ordering::SeqCst)
                && pump.target_ready.load(Ordering::SeqCst)
            {
                pump.phase = DndPumpPhase::MoveToTarget;
            }
        }
        DndPumpPhase::MoveToTarget => {
            keyboard_focus.focused_surface = Some(target_surface_id);
            apply_pointer_motion(
                &mut pointer,
                &mut pending_protocol_inputs,
                target_position.0,
                target_position.1,
            );
            pump.phase = DndPumpPhase::WaitForTargetOffer;
        }
        DndPumpPhase::WaitForTargetOffer => {
            let target_offer_negotiated =
                dnd_state.active_session.as_ref().is_some_and(|session| {
                    session.accepted_mime_type.as_deref() == Some(TEST_MIME_TYPE)
                        && session.chosen_action.is_some()
                });
            if target_offer_negotiated || pump.target_offer_ready.load(Ordering::SeqCst) {
                pump.phase = DndPumpPhase::ReleaseOnTarget;
            } else {
                keyboard_focus.focused_surface = Some(target_surface_id);
                apply_pointer_motion(
                    &mut pointer,
                    &mut pending_protocol_inputs,
                    target_position.0,
                    target_position.1,
                );
                pump.target_offer_attempts = pump.target_offer_attempts.saturating_add(1);
                if pump.target_offer_attempts >= 12 {
                    pump.phase = DndPumpPhase::ReleaseOnTarget;
                }
            }
        }
        DndPumpPhase::ReleaseOnTarget => {
            pending_protocol_inputs.push(BackendInputEvent {
                device: "dnd-test".to_owned(),
                action: BackendInputAction::PointerButton {
                    button_code: TEST_BUTTON_CODE,
                    pressed: false,
                },
            });
            pump.phase = DndPumpPhase::Done;
        }
        DndPumpPhase::Done => {}
    }
}

/// Applies one pointer motion event to the protocol input queue.
fn apply_pointer_motion(
    pointer: &mut GlobalPointerPosition,
    pending_protocol_inputs: &mut PendingProtocolInputEvents,
    x: f64,
    y: f64,
) {
    pointer.x = x;
    pointer.y = y;
    pending_protocol_inputs.extend([
        BackendInputEvent {
            device: "dnd-test".to_owned(),
            action: BackendInputAction::FocusChanged { focused: false },
        },
        BackendInputEvent {
            device: "dnd-test".to_owned(),
            action: BackendInputAction::FocusChanged { focused: true },
        },
        BackendInputEvent {
            device: "dnd-test".to_owned(),
            action: BackendInputAction::PointerMoved { x, y },
        },
    ]);
}

/// Picks a pointer coordinate guaranteed to fall inside the supplied geometry.
fn pointer_in_geometry(geometry: &SurfaceGeometry) -> (f64, f64) {
    // The compositor may allocate a larger window slot than the helper client's committed
    // 48x48 buffer. Keep the synthetic pointer inside that known buffer footprint so strict
    // surface-tree hit-testing still lands on the client surface.
    let x = f64::from(geometry.x)
        + f64::from((TEST_BUFFER_WIDTH / 2).min(geometry.width.saturating_sub(1)));
    let y = f64::from(geometry.y)
        + f64::from((TEST_BUFFER_HEIGHT / 2).min(geometry.height.saturating_sub(1)));
    (x, y)
}

fn geometry_matches(geometry: &SurfaceGeometry, x: i32, y: i32, width: u32, height: u32) -> bool {
    geometry.x == x && geometry.y == y && geometry.width == width && geometry.height == height
}

/// Creates a small SHM buffer with deterministic pixel data for the helper clients.
fn create_test_buffer<T>(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<T>,
    width: u32,
    height: u32,
) -> Result<(std::fs::File, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer), common::TestControl>
where
    T: Dispatch<wl_shm_pool::WlShmPool, ()> + Dispatch<wl_buffer::WlBuffer, ()> + 'static,
{
    let stride = width * 4;
    let file_size = stride * height;
    let mut file = tempfile().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    let mut pixels = vec![0_u8; file_size as usize];
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0x55, 0xaa, 0xdd, 0x00]);
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
        width as i32,
        height as i32,
        stride as i32,
        wl_shm::Format::Xrgb8888,
        qh,
        (),
    );

    Ok((file, pool, buffer))
}

/// Runs the source DnD client until it starts the drag and serves any requested payload reads.
fn run_source_client(
    socket_path: &Path,
    ready_flag: Arc<AtomicBool>,
    pointer_ready_flag: Arc<AtomicBool>,
    drag_started_flag: Arc<AtomicBool>,
) -> Result<SourceClientSummary, common::TestControl> {
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

    let mut state = SourceClientState::default();
    let deadline = Instant::now() + Duration::from_secs(3);

    while state.send_requests == 0 {
        dispatch_source_client_once(&mut event_queue, &mut state)?;
        if state.buffer_attached {
            ready_flag.store(true, Ordering::SeqCst);
        }
        if state.pointer_inside {
            pointer_ready_flag.store(true, Ordering::SeqCst);
        }
        if state.drag_started {
            drag_started_flag.store(true, Ordering::SeqCst);
        }
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for source DnD send request (configured={}, buffer_attached={}, pointer_bound={}, data_device_bound={}, pointer_inside={}, drag_started={})",
                state.configured,
                state.buffer_attached,
                state.pointer.is_some(),
                state.data_device.is_some(),
                state.pointer_inside,
                state.drag_started
            )));
        }
    }

    Ok(SourceClientSummary {
        globals: state.globals,
        drag_started: state.drag_started,
        send_requests: state.send_requests,
    })
}

/// Performs one read/dispatch cycle for the source DnD client.
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

/// Runs the target DnD client until it receives the drop payload.
fn run_target_client(
    socket_path: &Path,
    ready_flag: Arc<AtomicBool>,
    offer_ready_flag: Arc<AtomicBool>,
) -> Result<TargetClientSummary, common::TestControl> {
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

    let mut state = TargetClientState::default();
    let deadline = Instant::now() + Duration::from_secs(3);

    while state.received_payload != TEST_DND_BYTES {
        dispatch_target_client_once(&mut event_queue, &mut state)?;
        if state.buffer_attached && state.data_device.is_some() {
            ready_flag.store(true, Ordering::SeqCst);
        }
        if state.offered_test_mime {
            offer_ready_flag.store(true, Ordering::SeqCst);
        }
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for DnD payload (offer_present={}, accepted={}, receive_requested={}, enters={}, offers={}, drops={}, leaves={})",
                state.drag_offer.is_some(),
                state.offered_test_mime,
                state.receive_requested,
                state.enter_count,
                state.offer_count,
                state.drop_count,
                state.leave_count,
            )));
        }
    }

    Ok(TargetClientSummary { globals: state.globals, received_payload: state.received_payload })
}

/// Performs one read/dispatch cycle for the target DnD client.
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
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, 1, qh, ()));
                    state.maybe_bind_devices(qh);
                }
                "wl_data_device_manager" => {
                    state.data_device_manager =
                        Some(registry.bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(
                            name,
                            3,
                            qh,
                            (),
                        ));
                    state.maybe_bind_devices(qh);
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
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
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
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.attach_test_buffer(qh);
            if let Some(surface) = state.base_surface.as_ref() {
                surface.commit();
            }
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for SourceClientState {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.configured_width = u32::try_from(width).ok().filter(|width| *width > 0);
            state.configured_height = u32::try_from(height).ok().filter(|height| *height > 0);
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
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.attach_test_buffer(qh);
            if let Some(surface) = state.base_surface.as_ref() {
                surface.commit();
            }
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for TargetClientState {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.configured_width = u32::try_from(width).ok().filter(|width| *width > 0);
            state.configured_height = u32::try_from(height).ok().filter(|height| *height > 0);
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
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for TargetClientState {
    fn event(
        _state: &mut Self,
        _seat: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for SourceClientState {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => {
                state.pointer_inside = state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id());
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id())
                {
                    state.pointer_inside = false;
                }
            }
            wl_pointer::Event::Button {
                button,
                serial,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if button == TEST_BUTTON_CODE && state.pointer_inside && !state.drag_started => {
                state.start_drag(qh, serial);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for SourceClientState {
    event_created_child!(SourceClientState, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ())
    ]);

    fn event(
        _state: &mut Self,
        _data_device: &wl_data_device::WlDataDevice,
        _event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for TargetClientState {
    event_created_child!(TargetClientState, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ())
    ]);

    fn event(
        state: &mut Self,
        _data_device: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter { serial, surface, id, .. } => {
                let is_target_surface = state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id());
                if is_target_surface {
                    state.enter_count = state.enter_count.saturating_add(1);
                    state.enter_serial = Some(serial);
                    state.drag_offer = id;
                    let _ = state.maybe_accept_drag();
                }
            }
            wl_data_device::Event::Leave => {
                state.leave_count = state.leave_count.saturating_add(1);
                state.drag_offer = None;
                state.enter_serial = None;
                state.offered_test_mime = false;
            }
            wl_data_device::Event::Drop => {
                state.drop_count = state.drop_count.saturating_add(1);
                let _ = state.maybe_request_receive();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for SourceClientState {
    fn event(
        state: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } if mime_type == TEST_MIME_TYPE => {
                let mut file = std::fs::File::from(fd);
                if let Err(error) = file.write_all(TEST_DND_BYTES) {
                    panic!("source client should write drag-and-drop payload: {error}");
                }
                state.send_requests = state.send_requests.saturating_add(1);
            }
            wl_data_source::Event::Cancelled if state.data_source.as_ref() == Some(source) => {
                state.data_source = None;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for SourceClientState {
    fn event(
        _state: &mut Self,
        _offer: &wl_data_offer::WlDataOffer,
        _event: wl_data_offer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for TargetClientState {
    fn event(
        state: &mut Self,
        _offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } if mime_type == TEST_MIME_TYPE => {
                state.offer_count = state.offer_count.saturating_add(1);
                state.offered_test_mime = true;
                let _ = state.maybe_accept_drag();
            }
            wl_data_offer::Event::SourceActions { .. } => {
                let _ = state.maybe_accept_drag();
            }
            _ => {}
        }
    }
}

delegate_noop!(SourceClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(SourceClientState: ignore wl_surface::WlSurface);
delegate_noop!(SourceClientState: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(SourceClientState: ignore wl_shm::WlShm);
delegate_noop!(SourceClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(SourceClientState: ignore wl_buffer::WlBuffer);

delegate_noop!(TargetClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(TargetClientState: ignore wl_surface::WlSurface);
delegate_noop!(TargetClientState: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(TargetClientState: ignore wl_shm::WlShm);
delegate_noop!(TargetClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(TargetClientState: ignore wl_buffer::WlBuffer);

impl SourceClientState {
    /// Create the source helper toplevel once both compositor globals are available.
    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let Some(compositor) = self.compositor.as_ref() else {
            return;
        };
        let Some(wm_base) = self.wm_base.as_ref() else {
            return;
        };
        let base_surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title("dnd-source".to_owned());
        base_surface.commit();

        self.base_surface = Some(base_surface);
        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    /// Bind the seat-scoped data device once both the seat and manager are known.
    fn maybe_bind_devices(&mut self, qh: &QueueHandle<Self>) {
        if self.data_device.is_none() && self.data_device_manager.is_some() && self.seat.is_some() {
            let Some(manager) = self.data_device_manager.as_ref() else {
                return;
            };
            let Some(seat) = self.seat.as_ref() else {
                return;
            };
            self.data_device = Some(manager.get_data_device(seat, qh, ()));
        }
    }

    /// Start the drag with a single offered MIME type and mark the source as active.
    fn start_drag(&mut self, qh: &QueueHandle<Self>, serial: u32) {
        let Some(manager) = self.data_device_manager.as_ref() else {
            return;
        };
        let Some(data_device) = self.data_device.as_ref() else {
            return;
        };
        let Some(base_surface) = self.base_surface.as_ref() else {
            return;
        };

        let source = manager.create_data_source(qh, ());
        source.offer(TEST_MIME_TYPE.to_owned());
        source.set_actions(wl_data_device_manager::DndAction::Copy);
        data_device.start_drag(Some(&source), base_surface, None, serial);
        self.data_source = Some(source);
        self.drag_started = true;
    }

    /// Attach a real SHM buffer so the source surface can participate in pointer focus and DnD.
    fn attach_test_buffer(&mut self, qh: &QueueHandle<Self>) {
        if self.buffer_attached || self.shm.is_none() || self.base_surface.is_none() {
            return;
        }

        let Some(shm) = self.shm.as_ref() else {
            return;
        };
        let width = self.configured_width.unwrap_or(TEST_BUFFER_WIDTH).max(1);
        let height = self.configured_height.unwrap_or(TEST_BUFFER_HEIGHT).max(1);
        let (file, pool, buffer) = match create_test_buffer(shm, qh, width, height) {
            Ok(buffer) => buffer,
            Err(error) => panic!("source DnD client should create a wl_shm buffer: {error:?}"),
        };
        let Some(surface) = self.base_surface.as_ref() else {
            return;
        };
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
        self._backing_file = Some(file);
        self._pool = Some(pool);
        self._buffer = Some(buffer);
        self.buffer_attached = true;
    }
}

impl TargetClientState {
    /// Create the target helper toplevel once both compositor globals are available.
    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let Some(compositor) = self.compositor.as_ref() else {
            return;
        };
        let Some(wm_base) = self.wm_base.as_ref() else {
            return;
        };
        let base_surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        toplevel.set_title("dnd-target".to_owned());
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

        let Some(manager) = self.data_device_manager.as_ref() else {
            return;
        };
        let Some(seat) = self.seat.as_ref() else {
            return;
        };
        self.data_device = Some(manager.get_data_device(seat, qh, ()));
    }

    /// Accept the drag once the compositor provided both an enter serial and the expected MIME.
    fn maybe_accept_drag(&mut self) -> Result<(), common::TestControl> {
        let Some(offer) = self.drag_offer.as_ref() else {
            return Ok(());
        };
        let Some(serial) = self.enter_serial else {
            return Ok(());
        };
        if !self.offered_test_mime {
            return Ok(());
        }

        offer.accept(serial, Some(TEST_MIME_TYPE.to_owned()));
        offer.set_actions(
            wl_data_device_manager::DndAction::Copy,
            wl_data_device_manager::DndAction::Copy,
        );
        Ok(())
    }

    /// Start reading the accepted offer into a local pipe once the MIME negotiation finished.
    fn maybe_request_receive(&mut self) -> Result<(), common::TestControl> {
        if self.receive_requested || !self.offered_test_mime {
            return Ok(());
        }
        let Some(offer) = self.drag_offer.as_ref() else {
            return Ok(());
        };

        let (read_end, write_end) = std::os::unix::net::UnixStream::pair()
            .map_err(|error| common::TestControl::Fail(error.to_string()))?;
        read_end
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|error| common::TestControl::Fail(error.to_string()))?;
        offer.receive(TEST_MIME_TYPE.to_owned(), write_end.as_fd());
        drop(write_end);
        self.pending_read = Some(read_end);
        self.receive_requested = true;
        Ok(())
    }

    /// Poll the receive pipe and finalize the drop once bytes arrive.
    fn try_read_received_payload(&mut self) -> Result<(), common::TestControl> {
        let Some(read_end) = self.pending_read.as_mut() else {
            return Ok(());
        };

        let mut payload = Vec::new();
        match read_end.read_to_end(&mut payload) {
            Ok(0) => Ok(()),
            Ok(_) => {
                self.received_payload = payload;
                self.pending_read = None;
                if let Some(offer) = self.drag_offer.take() {
                    offer.finish();
                    offer.destroy();
                }
                Ok(())
            }
            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock) => Ok(()),
            Err(error) => Err(common::TestControl::Fail(error.to_string())),
        }
    }

    fn attach_test_buffer(&mut self, qh: &QueueHandle<Self>) {
        if self.buffer_attached || self.shm.is_none() || self.base_surface.is_none() {
            return;
        }

        let Some(shm) = self.shm.as_ref() else {
            return;
        };
        let width = self.configured_width.unwrap_or(TEST_BUFFER_WIDTH).max(1);
        let height = self.configured_height.unwrap_or(TEST_BUFFER_HEIGHT).max(1);
        let (file, pool, buffer) = match create_test_buffer(shm, qh, width, height) {
            Ok(buffer) => buffer,
            Err(error) => panic!("target DnD client should create a wl_shm buffer: {error:?}"),
        };
        let Some(surface) = self.base_surface.as_ref() else {
            return;
        };
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
        self._backing_file = Some(file);
        self._pool = Some(pool);
        self._buffer = Some(buffer);
        self.buffer_attached = true;
    }
}
