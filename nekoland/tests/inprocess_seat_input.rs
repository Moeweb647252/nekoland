//! In-process integration test for keyboard and pointer seat events reaching a real Wayland
//! client.

use std::io::Write;
use std::os::fd::AsFd;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::components::{SurfaceGeometry, WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompositorClock, GlobalPointerPosition,
    KeyboardFocusState, PendingProtocolInputEvents,
};
use nekoland_protocol::ProtocolServerState;
use nekoland_shell::decorations;
use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

/// Linux keycode used for the synthetic keyboard press.
const TEST_KEYCODE: u32 = 36;
/// Pointer button code used for the synthetic click.
const TEST_BUTTON_CODE: u32 = 0x110;
/// Maximum number of frames the synthetic seat-input pump stays active.
const INPUT_PUMP_FRAMES: u8 = 8;
/// Frame at which the synthetic seat-input pump starts driving input.
const INPUT_PUMP_START_FRAME: u64 = 6;

/// Synthetic input pump that drives seat activity for the scenario.
#[derive(Debug, Default, Resource)]
struct SeatInputPump {
    /// Remaining frames during which synthetic input will be injected.
    remaining_frames: u8,
    /// Tick counter used to alternate the pointer location within the surface.
    tick: u8,
}

/// Summary returned by the helper seat-input client.
#[derive(Debug, Default)]
struct SeatClientSummary {
    globals: Vec<String>,
    keyboard_enter_count: usize,
    pointer_enter_count: usize,
    pointer_motion_count: usize,
    key_press_count: usize,
    button_press_count: usize,
}

/// Helper Wayland client state used to create one toplevel and observe seat-enter, motion, key,
/// and button events.
#[derive(Debug, Default)]
struct SeatClientState {
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    shm: Option<wl_shm::WlShm>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    _pool: Option<wl_shm_pool::WlShmPool>,
    _buffer: Option<wl_buffer::WlBuffer>,
    _backing_file: Option<std::fs::File>,
    /// Last configure serial seen for the helper toplevel.
    configure_serial: Option<u32>,
    /// Whether the helper attached a real SHM buffer yet.
    buffer_attached: bool,
    /// Number of keyboard enter events observed on the helper surface.
    keyboard_enter_count: usize,
    /// Number of pointer enter events observed on the helper surface.
    pointer_enter_count: usize,
    /// Number of pointer motion events observed on the helper surface.
    pointer_motion_count: usize,
    /// Number of pressed key events observed by the helper client.
    key_press_count: usize,
    /// Number of pressed pointer-button events observed by the helper client.
    button_press_count: usize,
}

/// Verifies that synthetic seat input propagated through the protocol pipeline reaches a real
/// client.
#[test]
fn seat_input_events_reach_real_wayland_client() {
    let Some(summary) = run_seat_input_scenario() else {
        return;
    };

    common::assert_globals_present(&summary.globals);
    assert!(summary.keyboard_enter_count >= 1, "client should receive wl_keyboard.enter");
    assert!(summary.key_press_count >= 1, "client should receive wl_keyboard.key");
    assert!(summary.pointer_enter_count >= 1, "client should receive wl_pointer.enter");
    assert!(summary.pointer_motion_count >= 1, "client should receive wl_pointer.motion");
    assert!(summary.button_press_count >= 1, "client should receive wl_pointer.button");
}

/// Runs the seat-input scenario and returns the helper-client summary.
fn run_seat_input_scenario() -> Option<SeatClientSummary> {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _disable_startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-seat-input-runtime");
    let config_path =
        common::write_default_config_with_xwayland_disabled(&runtime_dir.path, "seat-input.toml");
    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });
    app.inner_mut()
        .insert_resource(SeatInputPump { remaining_frames: INPUT_PUMP_FRAMES, tick: 0 })
        .add_systems(
            LayoutSchedule,
            pump_protocol_seat_input.after(decorations::server_decoration_system),
        );

    let socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<ProtocolServerState>() else {
            panic!("protocol server state should be available immediately after build");
        };

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping seat input test in restricted environment: {error}");
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let client_thread = thread::spawn(move || run_seat_input_client(&socket_path));
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping seat input test in restricted environment: {reason}");
                return None;
            }
            Err(common::TestControl::Fail(reason)) => panic!("seat input client failed: {reason}"),
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };

    drop(runtime_dir);
    Some(summary)
}

/// Injects alternating pointer/key/button input into the protocol input queue.
fn pump_protocol_seat_input(
    clock: Res<CompositorClock>,
    mut pump: ResMut<SeatInputPump>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut pointer: ResMut<GlobalPointerPosition>,
    mut pending_protocol_inputs: ResMut<PendingProtocolInputEvents>,
    windows: Query<(&WlSurfaceHandle, &SurfaceGeometry), With<XdgWindow>>,
) {
    if pump.remaining_frames == 0 || clock.frame < INPUT_PUMP_START_FRAME {
        return;
    }

    let Some((surface, geometry)) = windows.iter().next() else {
        return;
    };

    keyboard_focus.focused_surface = Some(surface.id);

    let x_offset: f64 = if pump.tick.is_multiple_of(2) { 24.0 } else { 40.0 };
    let y_offset: f64 = if pump.tick.is_multiple_of(2) { 28.0 } else { 44.0 };
    let x = f64::from(geometry.x) + x_offset.min(f64::from(geometry.width.saturating_sub(1)));
    let y = f64::from(geometry.y) + y_offset.min(f64::from(geometry.height.saturating_sub(1)));
    pointer.x = x;
    pointer.y = y;

    pending_protocol_inputs.extend([
        BackendInputEvent {
            device: "seat-test".to_owned(),
            action: BackendInputAction::FocusChanged { focused: false },
        },
        BackendInputEvent {
            device: "seat-test".to_owned(),
            action: BackendInputAction::FocusChanged { focused: true },
        },
        BackendInputEvent {
            device: "seat-test".to_owned(),
            action: BackendInputAction::PointerMoved { x, y },
        },
        BackendInputEvent {
            device: "seat-test".to_owned(),
            action: BackendInputAction::Key { keycode: TEST_KEYCODE, pressed: true },
        },
        BackendInputEvent {
            device: "seat-test".to_owned(),
            action: BackendInputAction::PointerButton {
                button_code: TEST_BUTTON_CODE,
                pressed: true,
            },
        },
    ]);

    pump.remaining_frames = pump.remaining_frames.saturating_sub(1);
    pump.tick = pump.tick.saturating_add(1);
}

/// Runs the helper Wayland client until it has observed the expected seat events.
fn run_seat_input_client(socket_path: &Path) -> Result<SeatClientSummary, common::TestControl> {
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

    let mut state = SeatClientState::default();
    let deadline = Instant::now() + Duration::from_secs(2);

    while !state.is_complete() {
        dispatch_client_once(&mut event_queue, &mut state)?;
        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for seat input round-trip (configure={}, keyboard_enter={}, key_press={}, pointer_enter={}, pointer_motion={}, button_press={}, buffer_attached={})",
                state.configure_serial.is_some(),
                state.keyboard_enter_count,
                state.key_press_count,
                state.pointer_enter_count,
                state.pointer_motion_count,
                state.button_press_count,
                state.buffer_attached,
            )));
        }
    }

    Ok(SeatClientSummary {
        globals: state.globals,
        keyboard_enter_count: state.keyboard_enter_count,
        pointer_enter_count: state.pointer_enter_count,
        pointer_motion_count: state.pointer_motion_count,
        key_press_count: state.key_press_count,
        button_press_count: state.button_press_count,
    })
}

/// Performs one read/dispatch cycle for the helper seat-input client.
fn dispatch_client_once(
    event_queue: &mut EventQueue<SeatClientState>,
    state: &mut SeatClientState,
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

impl Dispatch<wl_registry::WlRegistry, ()> for SeatClientState {
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
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for SeatClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for SeatClientState {
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
            if let Some(surface) = state.base_surface.as_ref() {
                if !state.buffer_attached {
                    let Some(shm) = state.shm.as_ref() else {
                        panic!("wl_shm should be bound before the toplevel is configured");
                    };
                    let Ok((file, pool, buffer)) = create_test_buffer(shm, qh) else {
                        panic!("seat input client should create a wl_shm buffer");
                    };
                    surface.attach(Some(&buffer), 0, 0);
                    state._backing_file = Some(file);
                    state._pool = Some(pool);
                    state._buffer = Some(buffer);
                    state.buffer_attached = true;
                }
                surface.commit();
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for SeatClientState {
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
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for SeatClientState {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                if state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id())
                {
                    state.keyboard_enter_count += 1;
                }
            }
            wl_keyboard::Event::Key {
                state: WEnum::Value(wl_keyboard::KeyState::Pressed), ..
            } => {
                state.key_press_count += 1;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for SeatClientState {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => {
                if state
                    .base_surface
                    .as_ref()
                    .is_some_and(|base_surface| base_surface.id() == surface.id())
                {
                    state.pointer_enter_count += 1;
                }
            }
            wl_pointer::Event::Motion { .. } => {
                state.pointer_motion_count += 1;
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => {
                if button == TEST_BUTTON_CODE {
                    state.button_press_count += 1;
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(SeatClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(SeatClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(SeatClientState: ignore wl_surface::WlSurface);
delegate_noop!(SeatClientState: ignore wl_shm::WlShm);
delegate_noop!(SeatClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(SeatClientState: ignore xdg_toplevel::XdgToplevel);

impl SeatClientState {
    /// Creates the helper toplevel once both `wl_compositor` and `xdg_wm_base` are available.
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

    /// Indicates whether the helper client has observed the full seat-input round-trip.
    fn is_complete(&self) -> bool {
        self.configure_serial.is_some()
            && self.keyboard_enter_count > 0
            && self.key_press_count > 0
            && self.pointer_enter_count > 0
            && self.pointer_motion_count > 0
            && self.button_press_count > 0
    }
}

/// Creates a small SHM buffer so the helper client can present a real surface and receive seat
/// focus.
fn create_test_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<SeatClientState>,
) -> Result<(std::fs::File, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer), common::TestControl> {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 32;
    const STRIDE: u32 = WIDTH * 4;
    let file_size = (STRIDE * HEIGHT) as usize;

    let mut file = tempfile().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    let mut pixels = vec![0_u8; file_size];
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0x33, 0x66, 0x99, 0xff]);
    }
    file.write_all(&pixels).map_err(|error| common::TestControl::Fail(error.to_string()))?;
    file.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

    let pool = shm.create_pool(file.as_fd(), file_size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        WIDTH as i32,
        HEIGHT as i32,
        STRIDE as i32,
        wl_shm::Format::Xrgb8888,
        qh,
        (),
    );

    Ok((file, pool, buffer))
}
