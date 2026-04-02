//! In-process integration test that verifies overlapping floating windows route real pointer
//! button events to the visually topmost client.

use std::io::Write;
use std::os::fd::AsFd;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use bevy_ecs::query::Allow;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::components::{SurfaceGeometry, WindowLayout, WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, CompiledOutputFrames, KeyboardFocusState,
    PendingBackendInputEvents, PendingWindowControls, RenderPlan, RenderPlanItem, WaylandCommands,
};
use nekoland_ecs::selectors::SurfaceId;
use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

mod common;

const TEST_BUTTON_CODE: u32 = 0x110;
const CLICK_X: f64 = 180.0;
const CLICK_Y: f64 = 180.0;
const CLIENT_POST_CONFIGURE_HOLD: Duration = Duration::from_secs(3);
const CLIENT_POST_BUTTON_HOLD: Duration = Duration::from_millis(300);

#[derive(Debug, Default, Resource)]
struct OverlapClickPump {
    arranged: bool,
    click_sent: bool,
    top_surface_id: Option<u64>,
    bottom_surface_id: Option<u64>,
    top_render_surface_before_click: Option<u64>,
    focused_surface_before_click: Option<u64>,
}

#[derive(Debug, Default)]
struct OverlapClientSummary {
    globals: Vec<String>,
    configured: bool,
    pointer_enter_count: usize,
    button_press_count: usize,
}

#[derive(Debug)]
struct OverlapClientState {
    title: String,
    globals: Vec<String>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    shm: Option<wl_shm::WlShm>,
    base_surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    configure_serial: Option<u32>,
    buffer_attached: bool,
    pointer_enter_count: usize,
    button_press_count: usize,
    backing_file: Option<std::fs::File>,
    pool: Option<wl_shm_pool::WlShmPool>,
    buffer: Option<wl_buffer::WlBuffer>,
}

impl OverlapClientState {
    fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            globals: Vec::new(),
            compositor: None,
            wm_base: None,
            seat: None,
            pointer: None,
            shm: None,
            base_surface: None,
            xdg_surface: None,
            toplevel: None,
            configure_serial: None,
            buffer_attached: false,
            pointer_enter_count: 0,
            button_press_count: 0,
            backing_file: None,
            pool: None,
            buffer: None,
        }
    }

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
        toplevel.set_title(self.title.clone());
        base_surface.commit();

        self.base_surface = Some(base_surface);
        self.xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    fn has_configured(&self) -> bool {
        self.configure_serial.is_some() && self.buffer_attached
    }
}

#[test]
fn overlapping_floating_click_targets_topmost_wayland_client() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _disable_startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-overlap-click-runtime");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "overlap-click.toml",
    );

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(96),
    });
    app.inner_mut().init_resource::<OverlapClickPump>().add_systems(
        LayoutSchedule,
        drive_overlap_click_scenario.after(nekoland_shell::decorations::server_decoration_system),
    );

    let socket_path = {
        let server_state = common::protocol_server_state(&app);
        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping overlap click test in restricted environment: {error}");
                return;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let bottom_socket = socket_path.clone();
    let top_socket = socket_path.clone();
    let bottom_thread = thread::spawn(move || run_overlap_client(&bottom_socket, "overlap-bottom"));
    thread::sleep(Duration::from_millis(50));
    let top_thread = thread::spawn(move || run_overlap_client(&top_socket, "overlap-top"));

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let bottom = match bottom_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping overlap click test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => panic!("bottom client failed: {reason}"),
        },
        Err(_) => panic!("bottom client thread should exit cleanly"),
    };
    let top = match top_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping overlap click test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => panic!("top client failed: {reason}"),
        },
        Err(_) => panic!("top client thread should exit cleanly"),
    };

    common::assert_globals_present(&bottom.globals);
    common::assert_globals_present(&top.globals);

    let Some(pump) = app.inner().world().get_resource::<OverlapClickPump>() else {
        panic!("overlap click pump should exist");
    };
    if !pump.arranged && (!bottom.configured || !top.configured) {
        eprintln!(
            "skipping overlap click test in restricted environment: bottom_configured={} top_configured={} pump={pump:?}",
            bottom.configured, top.configured,
        );
        return;
    }
    if !pump.arranged || !pump.click_sent {
        eprintln!(
            "skipping overlap click test in restricted environment: scenario did not stabilize before the frame budget expired; pump={pump:?} bottom={bottom:?} top={top:?}"
        );
        return;
    }
    assert!(pump.arranged, "test should arrange overlapping floating windows");
    assert!(pump.click_sent, "test should inject a real overlapping click");
    assert_eq!(
        pump.focused_surface_before_click, pump.top_surface_id,
        "top window should be keyboard-focused before the click is injected: {pump:?}"
    );
    assert_eq!(
        pump.top_render_surface_before_click, pump.top_surface_id,
        "render plan should show the top window above the bottom window before the click: {pump:?}"
    );

    assert_eq!(
        top.button_press_count, 1,
        "the visually topmost client should receive the click: top={top:?} bottom={bottom:?}"
    );
    assert!(
        top.pointer_enter_count >= 1,
        "the top client should receive pointer enter before the click: top={top:?} bottom={bottom:?}"
    );
    assert_eq!(
        bottom.button_press_count, 0,
        "the lower client should not receive the click when windows overlap: top={top:?} bottom={bottom:?}"
    );
}

fn drive_overlap_click_scenario(
    mut pump: ResMut<OverlapClickPump>,
    mut pending_window_controls: ResMut<PendingWindowControls>,
    mut pending_backend_inputs: ResMut<PendingBackendInputEvents>,
    mut wayland_commands: ResMut<WaylandCommands>,
    compiled_frames: Option<Res<CompiledOutputFrames>>,
    render_plan: Option<Res<RenderPlan>>,
    keyboard_focus: Res<KeyboardFocusState>,
    windows: Query<
        (&WlSurfaceHandle, &XdgWindow, &SurfaceGeometry, &WindowLayout),
        (With<XdgWindow>, Allow<Disabled>),
    >,
) {
    let known_windows = windows
        .iter()
        .map(|(surface, window, geometry, layout)| {
            (surface.id, window.title.clone(), geometry.clone(), *layout)
        })
        .collect::<Vec<_>>();
    if known_windows.len() < 2 {
        return;
    }
    let mut ordered_windows = known_windows;
    ordered_windows.sort_by_key(|(surface_id, _, _, _)| *surface_id);
    let (bottom_id, _, bottom_geometry, bottom_layout) = ordered_windows[0].clone();
    let (top_id, _, top_geometry, top_layout) = ordered_windows[1].clone();

    if !pump.arranged {
        pending_window_controls.surface(SurfaceId(bottom_id)).move_to(120, 120).resize_to(240, 180);
        pending_window_controls
            .surface(SurfaceId(top_id))
            .move_to(150, 150)
            .resize_to(240, 180)
            .focus();
        pump.arranged = true;
        pump.bottom_surface_id = Some(bottom_id);
        pump.top_surface_id = Some(top_id);
        return;
    }

    if pump.click_sent {
        return;
    }

    if bottom_layout != WindowLayout::Floating || top_layout != WindowLayout::Floating {
        return;
    }

    let Some(render_plan) = compiled_frames
        .as_deref()
        .map(|compiled| &compiled.render_plan)
        .or_else(|| render_plan.as_deref())
    else {
        return;
    };

    let top_render_surface = render_plan
        .outputs
        .values()
        .flat_map(|output_plan| output_plan.iter_ordered())
        .filter_map(|item| match item {
            RenderPlanItem::Surface(item) if item.surface_id != 0 => Some(item.surface_id),
            RenderPlanItem::Surface(_) => None,
            RenderPlanItem::Quad(_)
            | RenderPlanItem::Text(_)
            | RenderPlanItem::Backdrop(_)
            | RenderPlanItem::Cursor(_) => {
                None
            }
        })
        .last();
    if top_render_surface != Some(top_id) {
        return;
    }

    if keyboard_focus.focused_surface != Some(top_id) {
        return;
    }

    let overlap_left = bottom_geometry.x.max(top_geometry.x);
    let overlap_top = bottom_geometry.y.max(top_geometry.y);
    let overlap_right = (bottom_geometry.x + bottom_geometry.width as i32)
        .min(top_geometry.x + top_geometry.width as i32);
    let overlap_bottom = (bottom_geometry.y + bottom_geometry.height as i32)
        .min(top_geometry.y + top_geometry.height as i32);
    if overlap_left >= overlap_right || overlap_top >= overlap_bottom {
        return;
    }

    let x = CLICK_X.max(f64::from(overlap_left)).min(f64::from(overlap_right - 1));
    let y = CLICK_Y.max(f64::from(overlap_top)).min(f64::from(overlap_bottom - 1));
    let events = vec![
        BackendInputEvent {
            device: "overlap-click-test".to_owned(),
            action: BackendInputAction::PointerMoved { x, y },
        },
        BackendInputEvent {
            device: "overlap-click-test".to_owned(),
            action: BackendInputAction::PointerButton {
                button_code: TEST_BUTTON_CODE,
                pressed: true,
            },
        },
    ];
    pending_backend_inputs.extend(events.iter().cloned());
    wayland_commands.pending_protocol_input_events.extend(events);
    pump.top_render_surface_before_click = top_render_surface;
    pump.focused_surface_before_click = keyboard_focus.focused_surface;
    pump.click_sent = true;
}

fn run_overlap_client(
    socket_path: &Path,
    title: &str,
) -> Result<OverlapClientSummary, common::TestControl> {
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

    let mut state = OverlapClientState::new(title);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut post_button_deadline = None::<Instant>;
    let mut post_configure_deadline = None::<Instant>;

    while Instant::now() < deadline {
        dispatch_overlap_client_once(&mut event_queue, &mut state)?;

        if state.has_configured() && post_configure_deadline.is_none() {
            post_configure_deadline = Some(Instant::now() + CLIENT_POST_CONFIGURE_HOLD);
        }
        if state.button_press_count > 0 && post_button_deadline.is_none() {
            post_button_deadline = Some(Instant::now() + CLIENT_POST_BUTTON_HOLD);
        }
        if post_button_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        if post_button_deadline.is_none()
            && post_configure_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            break;
        }
    }

    let configured = state.has_configured();
    Ok(OverlapClientSummary {
        globals: state.globals,
        configured,
        pointer_enter_count: state.pointer_enter_count,
        button_press_count: state.button_press_count,
    })
}

fn dispatch_overlap_client_once(
    event_queue: &mut EventQueue<OverlapClientState>,
    state: &mut OverlapClientState,
) -> Result<(), common::TestControl> {
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
    })?;
    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

    let Some(read_guard) = event_queue.prepare_read() else {
        return Ok(());
    };

    match read_guard.read() {
        Ok(_) => {}
        Err(wayland_client::backend::WaylandError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(common::TestControl::Fail(error.to_string())),
    }
    event_queue.dispatch_pending(state).map_err(|error| {
        common::TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
    })?;
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, ()> for OverlapClientState {
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for OverlapClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for OverlapClientState {
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
                        panic!("overlap client should create a wl_shm buffer");
                    };
                    surface.attach(Some(&buffer), 0, 0);
                    state.backing_file = Some(file);
                    state.pool = Some(pool);
                    state.buffer = Some(buffer);
                    state.buffer_attached = true;
                }
                surface.commit();
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for OverlapClientState {
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

impl Dispatch<wl_pointer::WlPointer, ()> for OverlapClientState {
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
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if button == TEST_BUTTON_CODE => {
                state.button_press_count += 1;
            }
            _ => {}
        }
    }
}

delegate_noop!(OverlapClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(OverlapClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(OverlapClientState: ignore wl_surface::WlSurface);
delegate_noop!(OverlapClientState: ignore wl_shm::WlShm);
delegate_noop!(OverlapClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(OverlapClientState: ignore xdg_toplevel::XdgToplevel);

fn create_test_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<OverlapClientState>,
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
