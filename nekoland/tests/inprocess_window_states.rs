//! In-process integration tests for a wide range of window and popup state transitions, including
//! fullscreen, maximize, minimize, popup grabs, workspace visibility, and IPC-driven close flows.

use std::io::ErrorKind;
use std::io::Write;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Query, ResMut, Resource, With};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::{LayoutSchedule, RenderSchedule};
use nekoland_ecs::components::{
    BufferState, OutputProperties, PopupGrab, SurfaceGeometry, WindowDisplayState, WindowLayout,
    WindowMode, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated};
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, FramePacingState, GlobalPointerPosition,
    KeyboardFocusState, RenderPlan, RenderPlanItem, WaylandCommands, WorkArea,
};
use nekoland_ecs::resources::{
    PendingPopupServerRequests, PendingWindowServerRequests, PopupServerAction, PopupServerRequest,
    WindowServerAction, WindowServerRequest,
};
use nekoland_ipc::commands::{
    PopupCommand, QueryCommand, TreeSnapshot, WindowCommand, WorkspaceCommand,
};
use nekoland_ipc::{
    IpcCommand, IpcReply, IpcRequest, IpcServerState, IpcSubscription, IpcSubscriptionEvent,
    SubscriptionTopic, send_request_to_path, subscribe_to_path,
};
use nekoland_shell::decorations;
use tempfile::tempfile;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};

mod common;

const INTERACTIVE_INPUT_PUMP_FRAMES: u8 = 8;
const CLIENT_LINGER_AFTER_COMPLETION: Duration = Duration::from_millis(400);
const SCENARIO_CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Enumerates the scenario variants exercised by this test module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowScenario {
    Maximize,
    RestoreMaximize,
    FullscreenPopup,
    RestoreFullscreen,
    Minimize,
    MoveResize,
    MoveResizeInvalidSerial,
    PopupGrab,
    ServerDismissGrabbedPopup,
    IpcDismissGrabbedPopup,
    PopupGrabInvalidSerial,
    PopupReposition,
    PopupDestroy,
    ToplevelDestroy,
    ServerCloseToplevel,
    IpcCloseToplevel,
    IpcCloseToplevelWithPopup,
    WorkspaceVisibility,
}

/// Summary returned by the helper client for one scenario run.
#[derive(Debug)]
struct ScenarioSummary {
    surface_configure_count: usize,
    popup_configure_serial: Option<u32>,
    popup_repositioned_token: Option<u32>,
    received_toplevel_close: bool,
    received_popup_done: bool,
    interactive_request_serial: Option<u32>,
}

/// Helper Wayland client state that drives toplevel/popup scenarios and records protocol
/// responses.
#[derive(Debug, Default)]
struct ScenarioClientState {
    scenario: Option<WindowScenario>,
    ipc_socket_path: Option<PathBuf>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    shm: Option<wl_shm::WlShm>,
    base_surface: Option<wl_surface::WlSurface>,
    toplevel_xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    toplevel_pool: Option<wl_shm_pool::WlShmPool>,
    toplevel_buffer: Option<wl_buffer::WlBuffer>,
    toplevel_backing_file: Option<std::fs::File>,
    popup_surface: Option<wl_surface::WlSurface>,
    popup_xdg_surface: Option<xdg_surface::XdgSurface>,
    popup: Option<xdg_popup::XdgPopup>,
    popup_pool: Option<wl_shm_pool::WlShmPool>,
    popup_buffer: Option<wl_buffer::WlBuffer>,
    popup_backing_file: Option<std::fs::File>,
    toplevel_configure_count: usize,
    popup_configure_serial: Option<u32>,
    popup_repositioned_token: Option<u32>,
    popup_configure_geometry: Option<(i32, i32, i32, i32)>,
    received_toplevel_close: bool,
    received_popup_done: bool,
    latest_pointer_button_serial: Option<u32>,
    interactive_request_serial: Option<u32>,
    scenario_stage: u8,
    final_request_sent: bool,
    buffer_attached: bool,
    popup_buffer_attached: bool,
    terminal_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PopupPositionerSpec {
    width: i32,
    height: i32,
    anchor_x: i32,
    anchor_y: i32,
    anchor_width: i32,
    anchor_height: i32,
    offset_x: i32,
    offset_y: i32,
}

/// Records `WindowClosed` messages emitted during scenarios that destroy/close toplevels.
#[derive(Debug, Default, Resource)]
struct ClosedWindowAudit {
    surface_ids: Vec<u64>,
}

/// Automatically issues a close request once a window appears in scenarios that need server-side
/// close behavior.
#[derive(Debug, Default, Resource)]
struct AutoCloseOnCreate {
    issued: bool,
}

/// Automatically issues a popup dismiss request in scenarios that need it.
#[derive(Debug, Default, Resource)]
struct AutoDismissPopup {
    issued: bool,
}

/// Synthetic input pump used by the interactive move/resize scenarios.
#[derive(Debug, Clone, Copy, Resource)]
struct InteractiveSeatInputPump {
    scenario: WindowScenario,
    remaining_frames: u8,
    tick: u8,
}

fn render_plan_surface_ids(world: &mut bevy_ecs::world::World) -> Vec<u64> {
    let Some(render_plan) = world.get_resource::<RenderPlan>() else {
        panic!("render plan should be initialized");
    };
    render_plan
        .outputs
        .values()
        .flat_map(|output_plan| output_plan.iter_ordered())
        .filter_map(|item| match item {
            RenderPlanItem::Surface(item) if item.surface_id != 0 => Some(item.surface_id),
            RenderPlanItem::Surface(_) => None,
            RenderPlanItem::SolidRect(_)
            | RenderPlanItem::Backdrop(_)
            | RenderPlanItem::Cursor(_) => None,
        })
        .collect()
}

#[test]
fn maximize_request_updates_window_state_and_geometry() {
    let Some((app, summary)) = run_scenario(WindowScenario::Maximize) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 2,
        "maximize should trigger a follow-up configure: {summary:?}"
    );

    let (window_state, geometry, output) = snapshot_window_and_output(app);
    assert_eq!(window_state, WindowDisplayState::Maximized);
    assert_eq!(geometry.x, 16);
    assert_eq!(geometry.y, 16);
    assert_eq!(geometry.width, output.width.saturating_sub(32).max(1));
    assert_eq!(geometry.height, output.height.saturating_sub(32).max(1));
}

#[test]
fn fullscreen_and_popup_requests_populate_popup_entity_and_render_plan() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::FullscreenPopup) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 2,
        "fullscreen should trigger a follow-up configure: {summary:?}"
    );
    assert!(
        summary.popup_configure_serial.is_some(),
        "popup scenario should receive a popup configure: {summary:?}"
    );

    let (
        window_surface_id,
        popup_surface_id,
        geometry,
        output,
        popup_parent,
        popup_grab_active,
        render_surface_ids,
    ) = {
        let world = app.inner_mut().world_mut();

        let mut window_query = world.query::<(
            &WlSurfaceHandle,
            &WindowLayout,
            &WindowMode,
            &SurfaceGeometry,
            &XdgWindow,
        )>();
        let windows = window_query
            .iter(world)
            .map(|(surface, layout, mode, geometry, _)| {
                (surface.id, WindowDisplayState::from_layout_mode(*layout, *mode), geometry.clone())
            })
            .collect::<Vec<_>>();
        assert_eq!(windows.len(), 1, "scenario should create exactly one toplevel window");
        let (window_surface_id, window_state, geometry) = windows[0].clone();
        assert_eq!(window_state, WindowDisplayState::Fullscreen);

        let output = world.query::<&OutputProperties>().iter(world).next().cloned();
        let Some(output) = output else {
            panic!("backend should create one output");
        };

        let mut popup_query = world.query::<(&WlSurfaceHandle, &PopupGrab, &ChildOf)>();
        let popups = popup_query
            .iter(world)
            .map(|(surface, grab, child_of)| (surface.id, child_of.parent(), grab.active))
            .collect::<Vec<_>>();
        assert_eq!(popups.len(), 1, "fullscreen popup scenario should create one popup entity");
        let (popup_surface_id, popup_parent_entity, popup_grab_active) = popups[0];
        let Some(popup_parent_surface) = world.get::<WlSurfaceHandle>(popup_parent_entity) else {
            panic!("popup parent should expose a surface handle");
        };
        let popup_parent = popup_parent_surface.id;

        let render_surface_ids = render_plan_surface_ids(world);

        (
            window_surface_id,
            popup_surface_id,
            geometry,
            output,
            popup_parent,
            popup_grab_active,
            render_surface_ids,
        )
    };

    assert_eq!(geometry.x, 0);
    assert_eq!(geometry.y, 0);
    assert_eq!(geometry.width, output.width.max(1));
    assert_eq!(geometry.height, output.height.max(1));
    assert_eq!(popup_parent, window_surface_id);
    assert!(!popup_grab_active, "popup scenario should create a non-grab popup by default");
    assert!(
        render_surface_ids.contains(&window_surface_id),
        "render plan should include the fullscreen window: {render_surface_ids:?}"
    );
    assert!(
        render_surface_ids.contains(&popup_surface_id),
        "render plan should include the popup surface: {render_surface_ids:?}"
    );
}

#[test]
fn unmaximize_request_restores_tiled_layout_geometry() {
    let Some((app, summary)) = run_scenario(WindowScenario::RestoreMaximize) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 2,
        "restore maximize should still observe the maximize configure: {summary:?}"
    );

    let (window_state, geometry, output, work_area) = snapshot_window_output_and_work_area(app);
    assert_eq!(window_state, WindowDisplayState::Floating);
    assert_eq!(geometry.width, 32, "restore maximize should recover the committed floating width");
    assert_eq!(
        geometry.height, 32,
        "restore maximize should recover the committed floating height"
    );
    assert_eq!(
        (geometry.x, geometry.y),
        centred_position(&work_area, &geometry),
        "restored floating geometry should be centred in the work area"
    );
    assert!(geometry.width < output.width);
    assert!(geometry.height < output.height);
}

#[test]
fn unfullscreen_request_restores_tiled_layout_geometry() {
    let Some((app, summary)) = run_scenario(WindowScenario::RestoreFullscreen) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 2,
        "restore fullscreen should still observe the fullscreen configure: {summary:?}"
    );
    let (window_state, geometry, output, work_area) = snapshot_window_output_and_work_area(app);
    assert_eq!(window_state, WindowDisplayState::Floating);
    assert_eq!(geometry.width, 32, "restore fullscreen should recover the committed width");
    assert_eq!(geometry.height, 32, "restore fullscreen should recover the committed height");
    assert_eq!(
        (geometry.x, geometry.y),
        centred_position(&work_area, &geometry),
        "restored floating geometry should be centred in the work area"
    );
    assert!(geometry.width < output.width);
    assert!(geometry.height < output.height);
}

#[test]
fn minimize_request_hides_window_clears_focus_and_removes_render_entry() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::Minimize) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 1,
        "minimize scenario should create the toplevel before minimizing: {summary:?}"
    );

    let (surface_id, state, focus, render_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let mut window_query =
            world.query::<(&WlSurfaceHandle, &WindowLayout, &WindowMode, &XdgWindow)>();
        let windows = window_query
            .iter(world)
            .map(|(surface, layout, mode, _)| {
                (surface.id, WindowDisplayState::from_layout_mode(*layout, *mode))
            })
            .collect::<Vec<_>>();
        assert_eq!(windows.len(), 1, "scenario should create exactly one toplevel window");
        let (surface_id, state) = windows[0];

        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus state should be initialized");
        };
        let focus = keyboard_focus.focused_surface;
        let render_surface_ids = render_plan_surface_ids(world);

        (surface_id, state, focus, render_surface_ids)
    };

    assert_eq!(state, WindowDisplayState::Hidden);
    assert_eq!(focus, None, "hidden window should not retain keyboard focus");
    assert!(
        !render_surface_ids.contains(&surface_id),
        "hidden window should be removed from the render plan: {render_surface_ids:?}"
    );
}

#[test]
fn interactive_move_request_switches_window_to_floating_geometry() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::MoveResize) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 1,
        "move+resize scenario should create a toplevel before requesting interaction: {summary:?}"
    );
    assert!(
        summary.interactive_request_serial.is_some(),
        "move+resize scenario should use a real wl_pointer button serial: {summary:?}"
    );

    let (surface_id, state, geometry, focus) = {
        let world = app.inner_mut().world_mut();
        let mut window_query = world.query::<(
            &WlSurfaceHandle,
            &WindowLayout,
            &WindowMode,
            &SurfaceGeometry,
            &XdgWindow,
        )>();
        let windows = window_query
            .iter(world)
            .map(|(surface, layout, mode, geometry, _)| {
                (surface.id, WindowDisplayState::from_layout_mode(*layout, *mode), geometry.clone())
            })
            .collect::<Vec<_>>();
        assert_eq!(windows.len(), 1, "scenario should create exactly one toplevel window");
        let (surface_id, state, geometry) = windows[0].clone();
        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus state should be initialized");
        };
        let focus = keyboard_focus.focused_surface;
        (surface_id, state, geometry, focus)
    };

    assert_eq!(state, WindowDisplayState::Floating);
    let Some(work_area) = app.inner().world().get_resource::<WorkArea>() else {
        panic!("work area should be initialized");
    };
    let expected_initial =
        centred_position(work_area, &SurfaceGeometry { x: 0, y: 0, width: 32, height: 32 });
    assert!(
        geometry.x > expected_initial.0,
        "interactive move should shift the floating window to the right: {geometry:?}"
    );
    assert!(
        geometry.y > expected_initial.1,
        "interactive move should shift the floating window downward: {geometry:?}"
    );
    assert_eq!(geometry.width, 32, "interactive move should preserve the committed width");
    assert_eq!(geometry.height, 32, "interactive move should preserve the committed height");
    assert_eq!(focus, Some(surface_id), "interactive move should focus the moved surface");
}

#[test]
fn move_and_resize_requests_with_invalid_serial_are_ignored() {
    let Some((app, summary)) = run_scenario(WindowScenario::MoveResizeInvalidSerial) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 1,
        "invalid move+resize scenario should still create the toplevel: {summary:?}"
    );
    assert_eq!(
        summary.interactive_request_serial, None,
        "invalid move+resize scenario should not consume a real pointer serial"
    );
    let (window_state, geometry, output, work_area) = snapshot_window_output_and_work_area(app);
    assert_eq!(window_state, WindowDisplayState::Floating);
    assert_eq!(geometry.width, 32, "invalid interactive requests should leave width unchanged");
    assert_eq!(geometry.height, 32, "invalid interactive requests should leave height unchanged");
    assert_eq!(
        (geometry.x, geometry.y),
        centred_position(&work_area, &geometry),
        "invalid interactive requests should leave the window centred"
    );
    assert!(geometry.width < output.width);
    assert!(geometry.height < output.height);
}

#[test]
fn popup_grab_request_marks_popup_active_and_tracks_serial() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::PopupGrab) else {
        return;
    };
    assert!(
        summary.popup_configure_serial.is_some(),
        "popup grab scenario should receive a popup configure before grabbing: {summary:?}"
    );
    assert!(
        summary.interactive_request_serial.is_some(),
        "popup grab scenario should use a real wl_pointer button serial: {summary:?}"
    );

    let (window_surface_id, popup_parent, popup_grab_serial, popup_configure_serial, grab) = {
        let world = app.inner_mut().world_mut();

        let window_surface_id = world
            .query::<(&WlSurfaceHandle, &XdgWindow)>()
            .iter(world)
            .map(|(surface, _)| surface.id)
            .next()
            .unwrap_or_else(|| panic!("scenario should create a toplevel surface"));

        let mut popup_query = world.query::<(&XdgPopup, &PopupGrab, &ChildOf)>();
        let popups = popup_query
            .iter(world)
            .map(|(popup, grab, child_of)| {
                (child_of.parent(), popup.grab_serial, popup.configure_serial, grab.clone())
            })
            .collect::<Vec<_>>();
        assert_eq!(popups.len(), 1, "scenario should create exactly one popup");
        let (popup_parent_entity, popup_grab_serial, popup_configure_serial, grab) =
            popups[0].clone();
        let Some(popup_parent_surface) = world.get::<WlSurfaceHandle>(popup_parent_entity) else {
            panic!("popup parent should expose a surface handle");
        };
        let popup_parent = popup_parent_surface.id;

        (window_surface_id, popup_parent, popup_grab_serial, popup_configure_serial, grab)
    };

    assert_eq!(popup_grab_serial, summary.interactive_request_serial);
    assert_eq!(popup_configure_serial, summary.popup_configure_serial);
    assert_eq!(popup_parent, window_surface_id);
    assert!(grab.active, "popup grab should become active after popup.grab");
    assert_eq!(grab.seat_id, nekoland_ecs::components::SeatId::PRIMARY);
    assert_eq!(grab.serial, summary.interactive_request_serial);
}

#[test]
fn server_dismiss_of_grabbed_popup_sends_popup_done_and_cleans_up_popup_state() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::ServerDismissGrabbedPopup) else {
        return;
    };
    assert!(
        summary.popup_configure_serial.is_some(),
        "server popup dismiss scenario should configure the popup before grabbing: {summary:?}"
    );
    assert!(
        summary.interactive_request_serial.is_some(),
        "server popup dismiss scenario should use a real wl_pointer button serial: {summary:?}"
    );
    assert!(
        summary.received_popup_done,
        "server popup dismiss should notify the client with popup_done: {summary:?}"
    );

    let (popup_count, window_count, render_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let render_surface_ids = render_plan_surface_ids(world);
        (popup_count, window_count, render_surface_ids)
    };

    assert_eq!(popup_count, 0, "server popup dismiss should remove the popup entity");
    assert_eq!(window_count, 1, "server popup dismiss should keep the toplevel alive");
    assert_eq!(
        render_surface_ids.len(),
        1,
        "render plan should only contain the toplevel after server popup dismissal"
    );
}

#[test]
fn ipc_dismiss_of_grabbed_popup_sends_popup_done_and_cleans_up_popup_state() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::IpcDismissGrabbedPopup) else {
        return;
    };
    assert!(
        summary.popup_configure_serial.is_some(),
        "IPC popup dismiss scenario should configure the popup before grabbing: {summary:?}"
    );
    assert!(
        summary.interactive_request_serial.is_some(),
        "IPC popup dismiss scenario should use a real wl_pointer button serial: {summary:?}"
    );
    assert!(
        summary.received_popup_done,
        "IPC popup dismiss should notify the client with popup_done: {summary:?}"
    );

    let (popup_count, window_count, render_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let render_surface_ids = render_plan_surface_ids(world);
        (popup_count, window_count, render_surface_ids)
    };

    assert_eq!(popup_count, 0, "IPC popup dismiss should remove the popup entity");
    assert_eq!(window_count, 1, "IPC popup dismiss should keep the toplevel alive");
    assert_eq!(
        render_surface_ids.len(),
        1,
        "render plan should only contain the toplevel after IPC popup dismissal"
    );
}

#[test]
fn popup_grab_request_with_invalid_serial_is_dismissed() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::PopupGrabInvalidSerial) else {
        return;
    };
    assert!(
        summary.popup_configure_serial.is_some(),
        "invalid popup grab scenario should receive a popup configure: {summary:?}"
    );
    assert!(
        summary.received_popup_done,
        "invalid popup grab should be rejected by dismissing the popup: {summary:?}"
    );
    assert_eq!(
        summary.interactive_request_serial, None,
        "invalid popup grab scenario should not consume a real pointer serial"
    );

    let (popup_count, window_count, render_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let render_surface_ids = render_plan_surface_ids(world);
        (popup_count, window_count, render_surface_ids)
    };

    assert_eq!(popup_count, 0, "invalid popup grab should remove the popup entity");
    assert_eq!(window_count, 1, "invalid popup grab should not remove the toplevel");
    assert_eq!(
        render_surface_ids.len(),
        1,
        "render plan should only contain the toplevel after popup dismissal"
    );
}

#[test]
fn popup_reposition_request_updates_geometry_and_token() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::PopupReposition) else {
        return;
    };
    assert_eq!(
        summary.popup_repositioned_token,
        Some(91),
        "popup reposition scenario should observe the repositioned event: {summary:?}"
    );

    let (parent_geometry, popup, geometry) = {
        let world = app.inner_mut().world_mut();
        let parent_geometry = world
            .query::<(&WlSurfaceHandle, &SurfaceGeometry, &XdgWindow)>()
            .iter(world)
            .map(|(_, geometry, _)| geometry.clone())
            .next()
            .unwrap_or_else(|| panic!("popup reposition scenario should keep the toplevel alive"));
        let mut popup_query = world.query::<(&XdgPopup, &SurfaceGeometry)>();
        let popups = popup_query
            .iter(world)
            .map(|(popup, geometry)| (popup.clone(), geometry.clone()))
            .collect::<Vec<_>>();
        assert_eq!(popups.len(), 1, "scenario should keep exactly one popup after reposition");
        (parent_geometry, popups[0].0.clone(), popups[0].1.clone())
    };

    assert_eq!(popup.reposition_token, Some(91));
    assert_eq!(geometry.x, parent_geometry.x + 100);
    assert_eq!(geometry.y, parent_geometry.y + 64);
    assert_eq!(geometry.width, 300);
    assert_eq!(geometry.height, 140);
}

#[test]
fn popup_destroy_request_removes_popup_entity_and_render_entry() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::PopupDestroy) else {
        return;
    };
    assert!(
        summary.popup_configure_serial.is_some(),
        "popup destroy scenario should configure the popup before destroying it: {summary:?}"
    );

    let (popup_count, render_surface_ids, window_count) = {
        let world = app.inner_mut().world_mut();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let render_surface_ids = render_plan_surface_ids(world);
        (popup_count, render_surface_ids, window_count)
    };

    assert_eq!(popup_count, 0, "popup entity should be removed after xdg_popup.destroy");
    assert_eq!(window_count, 1, "destroying a popup should not remove the toplevel window");
    assert_eq!(
        render_surface_ids.len(),
        1,
        "render plan should only contain the toplevel after popup destroy"
    );
}

#[test]
fn toplevel_destroy_removes_window_records_close_and_clears_render_focus() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::ToplevelDestroy) else {
        return;
    };
    assert!(
        summary.surface_configure_count >= 1,
        "toplevel destroy scenario should configure the toplevel before destroying it: {summary:?}"
    );

    let (window_count, popup_count, focus, render_surface_ids, closed_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus state should be initialized");
        };
        let focus = keyboard_focus.focused_surface;
        let render_surface_ids = render_plan_surface_ids(world);
        let Some(closed_window_audit) = world.get_resource::<ClosedWindowAudit>() else {
            panic!("closed window audit should be initialized");
        };
        let closed_surface_ids = closed_window_audit.surface_ids.clone();

        (window_count, popup_count, focus, render_surface_ids, closed_surface_ids)
    };

    assert_eq!(window_count, 0, "destroyed toplevel should be removed from ECS");
    assert_eq!(popup_count, 0, "destroying the only toplevel should not leave stray popups");
    assert_eq!(focus, None, "destroyed toplevel should clear keyboard focus");
    assert_eq!(
        render_surface_ids.len(),
        0,
        "destroyed toplevel should be removed from render plan. Elements left: {:?}",
        render_surface_ids
    );
    assert_eq!(closed_surface_ids.len(), 1, "destroy path should emit one WindowClosed message");
}

#[test]
fn server_close_request_emits_close_event_and_cleans_up_window() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::ServerCloseToplevel) else {
        return;
    };
    assert!(
        summary.received_toplevel_close,
        "client should receive xdg_toplevel.close in server-close scenario: {summary:?}"
    );

    let (window_count, focus, render_surface_ids, closed_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus state should be initialized");
        };
        let focus = keyboard_focus.focused_surface;
        let render_surface_ids = render_plan_surface_ids(world);
        let Some(closed_window_audit) = world.get_resource::<ClosedWindowAudit>() else {
            panic!("closed window audit should be initialized");
        };
        let closed_surface_ids = closed_window_audit.surface_ids.clone();

        (window_count, focus, render_surface_ids, closed_surface_ids)
    };

    assert_eq!(window_count, 0, "client should destroy the toplevel after receiving close");
    assert_eq!(focus, None, "closed toplevel should not retain keyboard focus");
    assert!(render_surface_ids.is_empty(), "closed toplevel should be removed from render plan");
    assert_eq!(
        closed_surface_ids.len(),
        1,
        "server-initiated close should still emit WindowClosed"
    );
}

#[test]
fn ipc_close_request_emits_close_event_and_cleans_up_window() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::IpcCloseToplevel) else {
        return;
    };
    assert!(
        summary.received_toplevel_close,
        "IPC close scenario should receive xdg_toplevel.close: {summary:?}"
    );

    let (window_count, focus, render_surface_ids, closed_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let window_count = world.query::<(&WlSurfaceHandle, &XdgWindow)>().iter(world).count();
        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus should remain available");
        };
        let focus = keyboard_focus.focused_surface;
        let render_surface_ids = render_plan_surface_ids(world);
        let Some(closed_window_audit) = world.get_resource::<ClosedWindowAudit>() else {
            panic!("closed window audit should be initialized");
        };
        let closed_surface_ids = closed_window_audit.surface_ids.clone();

        (window_count, focus, render_surface_ids, closed_surface_ids)
    };

    assert_eq!(window_count, 0, "IPC close should remove the toplevel window entity");
    assert_eq!(focus, None, "IPC close should clear keyboard focus");
    assert!(
        render_surface_ids.is_empty(),
        "IPC close should remove the window from the render plan"
    );
    assert_eq!(closed_surface_ids.len(), 1, "IPC close should emit exactly one WindowClosed");
}

#[test]
fn ipc_close_of_parent_window_dismisses_child_popup_and_cleans_up_everything() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::IpcCloseToplevelWithPopup) else {
        return;
    };
    assert!(
        summary.popup_configure_serial.is_some(),
        "IPC close with popup should configure the popup before close: {summary:?}"
    );
    assert!(
        summary.received_toplevel_close,
        "IPC close with popup should still deliver xdg_toplevel.close: {summary:?}"
    );
    assert!(
        summary.received_popup_done,
        "closing a parent window should dismiss its popup over protocol: {summary:?}"
    );

    let (window_count, popup_count, focus, render_surface_ids, closed_surface_ids) = {
        let world = app.inner_mut().world_mut();
        let window_count = world.query::<(&WlSurfaceHandle, &XdgWindow)>().iter(world).count();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus should remain available");
        };
        let focus = keyboard_focus.focused_surface;
        let render_surface_ids = render_plan_surface_ids(world);
        let Some(closed_window_audit) = world.get_resource::<ClosedWindowAudit>() else {
            panic!("closed window audit should be initialized");
        };
        let closed_surface_ids = closed_window_audit.surface_ids.clone();

        (window_count, popup_count, focus, render_surface_ids, closed_surface_ids)
    };

    assert_eq!(window_count, 0, "IPC close should remove the parent toplevel entity");
    assert_eq!(popup_count, 0, "IPC close should also remove child popup entities");
    assert_eq!(focus, None, "IPC close with popup should clear keyboard focus");
    assert!(render_surface_ids.is_empty(), "IPC close with popup should clear the render plan");
    assert_eq!(closed_surface_ids.len(), 1, "IPC close should emit one WindowClosed");
}

#[test]
fn ipc_subscription_stream_reports_popup_dismiss_and_tree_change_on_parent_close() {
    let subscription = IpcSubscription {
        topic: SubscriptionTopic::All,
        include_payloads: true,
        events: vec!["popup_dismissed".to_owned(), "tree_changed".to_owned()],
    };
    let Some((_app, summary, events)) =
        run_scenario_with_subscription(WindowScenario::IpcCloseToplevelWithPopup, subscription)
    else {
        return;
    };

    assert!(
        summary.received_toplevel_close,
        "subscription scenario should still deliver xdg_toplevel.close: {summary:?}"
    );
    assert!(
        summary.received_popup_done,
        "subscription scenario should dismiss the popup over protocol: {summary:?}"
    );
    assert!(
        events.iter().any(|event| {
            event.topic == SubscriptionTopic::Popup && event.event == "popup_dismissed"
        }),
        "subscription stream should emit a popup_dismissed event: {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| matches!(event.event.as_str(), "popup_dismissed" | "tree_changed")),
        "subscription stream should suppress events outside the requested event filters: {events:?}"
    );
    assert!(
        events.iter().any(|event| {
            event.topic == SubscriptionTopic::Tree
                && event.event == "tree_changed"
                && event
                    .payload
                    .clone()
                    .and_then(|payload| serde_json::from_value::<TreeSnapshot>(payload).ok())
                    .is_some_and(|tree| tree.windows.is_empty() && tree.popups.is_empty())
        }),
        "subscription stream should emit a tree_changed event for the empty final tree: {events:?}"
    );
}

#[test]
fn workspace_switch_dismisses_popups_and_reconfigures_reactivated_toplevels() {
    let Some((mut app, summary)) = run_scenario(WindowScenario::WorkspaceVisibility) else {
        return;
    };
    assert!(
        summary.received_popup_done,
        "workspace switch should dismiss the popup over protocol: {summary:?}"
    );
    assert!(
        summary.surface_configure_count >= 2,
        "switching back to the active workspace should reconfigure the toplevel: {summary:?}"
    );

    let (popup_count, window_count, render_surface_ids, active_workspaces, frame_pacing) = {
        let world = app.inner_mut().world_mut();
        let popup_count = world.query::<&XdgPopup>().iter(world).count();
        let window_count = world.query::<&XdgWindow>().iter(world).count();
        let render_surface_ids = render_plan_surface_ids(world);
        let active_workspaces = world
            .query::<&nekoland_ecs::components::Workspace>()
            .iter(world)
            .filter(|workspace| workspace.active)
            .map(|workspace| workspace.id.0)
            .collect::<Vec<_>>();
        let Some(frame_pacing) = world.get_resource::<FramePacingState>() else {
            panic!("frame pacing state should be initialized");
        };
        let frame_pacing = frame_pacing.clone();

        (popup_count, window_count, render_surface_ids, active_workspaces, frame_pacing)
    };

    assert_eq!(popup_count, 0, "popup should be gone after popup_done-driven teardown");
    assert_eq!(window_count, 1, "workspace visibility changes should not destroy the toplevel");
    assert_eq!(active_workspaces, vec![1], "workspace 1 should be active again after switch back");
    assert_eq!(
        render_surface_ids.len(),
        1,
        "only the reactivated toplevel should remain renderable after popup dismissal"
    );
    assert_eq!(
        frame_pacing.callback_surface_ids.len(),
        1,
        "only the toplevel should continue receiving frame callbacks after popup dismissal"
    );
    assert_eq!(
        frame_pacing.presentation_surface_ids, frame_pacing.callback_surface_ids,
        "presentation feedback should only target the visible toplevel after popup dismissal"
    );
    assert!(
        frame_pacing.throttled_surface_ids.is_empty(),
        "no hidden surfaces should remain after popup teardown: {frame_pacing:?}"
    );
}

/// Run one scenario end-to-end without an IPC subscription sidecar and return
/// the finished app plus client-observed summary.
fn run_scenario(
    scenario: WindowScenario,
) -> Option<(nekoland_core::prelude::NekolandApp, ScenarioSummary)> {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poison| poison.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-window-state-runtime");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "window-states.toml",
    );

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(128),
    });
    app.inner_mut()
        .init_resource::<ClosedWindowAudit>()
        .add_systems(RenderSchedule, capture_window_closed_messages);
    if matches!(scenario, WindowScenario::ServerCloseToplevel) {
        app.inner_mut()
            .init_resource::<AutoCloseOnCreate>()
            .add_systems(RenderSchedule, request_server_close_on_window_created);
    }
    if matches!(
        scenario,
        WindowScenario::MoveResize
            | WindowScenario::PopupGrab
            | WindowScenario::ServerDismissGrabbedPopup
            | WindowScenario::IpcDismissGrabbedPopup
    ) {
        app.insert_resource(GlobalPointerPosition { x: 128.0, y: 96.0 });
        app.inner_mut()
            .insert_resource(InteractiveSeatInputPump {
                scenario,
                remaining_frames: INTERACTIVE_INPUT_PUMP_FRAMES,
                tick: 0,
            })
            .add_systems(
                LayoutSchedule,
                pump_interactive_seat_input.after(decorations::server_decoration_system),
            );
    }
    if matches!(scenario, WindowScenario::ServerDismissGrabbedPopup) {
        app.inner_mut()
            .init_resource::<AutoDismissPopup>()
            .add_systems(RenderSchedule, request_server_popup_dismiss);
    }

    let socket_path = {
        let server_state = common::protocol_server_state(&app);
        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping window state test in restricted environment: {error}");
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping window state IPC test in restricted environment: {error}");
                return None;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let ipc_client_socket_path = ipc_socket_path.clone();
    let client_thread = thread::spawn(move || {
        run_scenario_client(&socket_path, scenario, Some(ipc_client_socket_path))
    });
    let ipc_thread = match scenario {
        WindowScenario::IpcCloseToplevel => {
            Some(thread::spawn(move || request_close_over_ipc(&ipc_socket_path)))
        }
        WindowScenario::IpcCloseToplevelWithPopup => {
            Some(thread::spawn(move || request_close_over_ipc_when_popup_visible(&ipc_socket_path)))
        }
        WindowScenario::IpcDismissGrabbedPopup => {
            Some(thread::spawn(move || request_popup_dismiss_over_ipc(&ipc_socket_path)))
        }
        _ => None,
    };
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping window state test in restricted environment: {reason}");
                return None;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("scenario client failed: {reason}");
            }
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };
    if let Some(ipc_thread) = ipc_thread {
        match ipc_thread.join() {
            Ok(result) => match result {
                Ok(_) => {}
                Err(common::TestControl::Skip(reason)) => {
                    eprintln!("skipping window state IPC test in restricted environment: {reason}");
                    return None;
                }
                Err(common::TestControl::Fail(reason)) => {
                    panic!("IPC scenario client failed: {reason}");
                }
            },
            Err(_) => panic!("IPC thread should exit cleanly"),
        }
    }

    drop(runtime_dir);
    Some((app, summary))
}

/// Variant of [`run_scenario`] that also collects IPC subscription events while
/// the scenario executes.
fn run_scenario_with_subscription(
    scenario: WindowScenario,
    subscription: IpcSubscription,
) -> Option<(nekoland_core::prelude::NekolandApp, ScenarioSummary, Vec<IpcSubscriptionEvent>)> {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poison| poison.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-window-state-runtime");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "window-states-subscribe.toml",
    );

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(128),
    });
    app.inner_mut()
        .init_resource::<ClosedWindowAudit>()
        .add_systems(RenderSchedule, capture_window_closed_messages);
    if matches!(scenario, WindowScenario::ServerCloseToplevel) {
        app.inner_mut()
            .init_resource::<AutoCloseOnCreate>()
            .add_systems(RenderSchedule, request_server_close_on_window_created);
    }
    if matches!(
        scenario,
        WindowScenario::MoveResize
            | WindowScenario::PopupGrab
            | WindowScenario::ServerDismissGrabbedPopup
            | WindowScenario::IpcDismissGrabbedPopup
    ) {
        app.insert_resource(GlobalPointerPosition { x: 128.0, y: 96.0 });
        app.inner_mut()
            .insert_resource(InteractiveSeatInputPump {
                scenario,
                remaining_frames: INTERACTIVE_INPUT_PUMP_FRAMES,
                tick: 0,
            })
            .add_systems(
                LayoutSchedule,
                pump_interactive_seat_input.after(decorations::server_decoration_system),
            );
    }
    if matches!(scenario, WindowScenario::ServerDismissGrabbedPopup) {
        app.inner_mut()
            .init_resource::<AutoDismissPopup>()
            .add_systems(RenderSchedule, request_server_popup_dismiss);
    }

    let socket_path = {
        let server_state = common::protocol_server_state(&app);
        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping window state test in restricted environment: {error}");
                return None;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping window state IPC test in restricted environment: {error}");
                return None;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let ipc_client_socket_path = ipc_socket_path.clone();
    let client_thread = thread::spawn(move || {
        run_scenario_client(&socket_path, scenario, Some(ipc_client_socket_path))
    });
    let subscription_socket_path = ipc_socket_path.clone();
    let subscription_thread =
        thread::spawn(move || collect_subscription_events(&subscription_socket_path, subscription));
    let ipc_thread = match scenario {
        WindowScenario::IpcCloseToplevel => {
            Some(thread::spawn(move || request_close_over_ipc(&ipc_socket_path)))
        }
        WindowScenario::IpcCloseToplevelWithPopup => {
            Some(thread::spawn(move || request_close_over_ipc_when_popup_visible(&ipc_socket_path)))
        }
        WindowScenario::IpcDismissGrabbedPopup => {
            Some(thread::spawn(move || request_popup_dismiss_over_ipc(&ipc_socket_path)))
        }
        _ => None,
    };
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping window state test in restricted environment: {reason}");
                return None;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("scenario client failed: {reason}");
            }
        },
        Err(_) => panic!("client thread should exit cleanly"),
    };
    let events = match subscription_thread.join() {
        Ok(result) => match result {
            Ok(events) => events,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!(
                    "skipping window state subscription test in restricted environment: {reason}"
                );
                return None;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("subscription client failed: {reason}");
            }
        },
        Err(_) => panic!("subscription thread should exit cleanly"),
    };
    if let Some(ipc_thread) = ipc_thread {
        match ipc_thread.join() {
            Ok(result) => match result {
                Ok(_) => {}
                Err(common::TestControl::Skip(reason)) => {
                    eprintln!("skipping window state IPC test in restricted environment: {reason}");
                    return None;
                }
                Err(common::TestControl::Fail(reason)) => {
                    panic!("IPC scenario client failed: {reason}");
                }
            },
            Err(_) => panic!("IPC thread should exit cleanly"),
        }
    }

    drop(runtime_dir);
    Some((app, summary, events))
}

/// Minimal Wayland client used by the window-state scenarios.
///
/// It drives one toplevel plus optional popup objects, records protocol
/// callbacks, and terminates once the selected scenario reaches its expected
/// final state.
fn run_scenario_client(
    socket_path: &std::path::Path,
    scenario: WindowScenario,
    ipc_socket_path: Option<PathBuf>,
) -> Result<ScenarioSummary, common::TestControl> {
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

    let mut state =
        ScenarioClientState { scenario: Some(scenario), ipc_socket_path, ..Default::default() };
    let deadline = std::time::Instant::now() + SCENARIO_CLIENT_TIMEOUT;

    while !state.is_complete() {
        event_queue.dispatch_pending(&mut state).map_err(|error| {
            common::TestControl::Fail(format!("dispatch_pending before read failed: {error}"))
        })?;
        event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;

        if let Some(read_guard) = event_queue.prepare_read() {
            read_guard.read().map_err(|error| common::TestControl::Fail(error.to_string()))?;
            event_queue.dispatch_pending(&mut state).map_err(|error| {
                common::TestControl::Fail(format!("dispatch_pending after read failed: {error}"))
            })?;
        }

        if std::time::Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for scenario completion: scenario={:?}, stage={}, toplevel_configures={}, popup_configure_serial={:?}, popup_repositioned_token={:?}, received_toplevel_close={}, received_popup_done={}, final_request_sent={}, terminal_error={:?}",
                state.scenario,
                state.scenario_stage,
                state.toplevel_configure_count,
                state.popup_configure_serial,
                state.popup_repositioned_token,
                state.received_toplevel_close,
                state.received_popup_done,
                state.final_request_sent,
                state.terminal_error,
            )));
        }

        if let Some(error) = state.terminal_error.take() {
            return Err(common::TestControl::Fail(error));
        }
    }

    event_queue.flush().map_err(|error| common::TestControl::Fail(error.to_string()))?;
    thread::sleep(CLIENT_LINGER_AFTER_COMPLETION);

    Ok(ScenarioSummary {
        surface_configure_count: state.toplevel_configure_count,
        popup_configure_serial: state.popup_configure_serial,
        popup_repositioned_token: state.popup_repositioned_token,
        received_toplevel_close: state.received_toplevel_close,
        received_popup_done: state.received_popup_done,
        interactive_request_serial: state.interactive_request_serial,
    })
}

/// Inject synthetic pointer and focus events for scenarios that need a valid
/// interactive serial or a small move/resize gesture.
fn pump_interactive_seat_input(
    mut pump: ResMut<InteractiveSeatInputPump>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut pointer: ResMut<GlobalPointerPosition>,
    mut wayland_commands: ResMut<WaylandCommands>,
    windows: Query<(&WlSurfaceHandle, &SurfaceGeometry, &BufferState), With<XdgWindow>>,
) {
    if pump.remaining_frames == 0 {
        return;
    }

    let Some((surface, geometry, buffer)) = windows.iter().next() else {
        return;
    };
    if !buffer.attached {
        return;
    };

    keyboard_focus.focused_surface = Some(surface.id);
    let device = match pump.scenario {
        WindowScenario::MoveResize => "move-resize-test",
        WindowScenario::PopupGrab => "popup-grab-test",
        _ => "interactive-seat-test",
    };

    let mut events = vec![
        BackendInputEvent {
            device: device.to_owned(),
            action: BackendInputAction::FocusChanged { focused: false },
        },
        BackendInputEvent {
            device: device.to_owned(),
            action: BackendInputAction::FocusChanged { focused: true },
        },
    ];

    let (x, y, pressed) = if matches!(pump.scenario, WindowScenario::MoveResize) {
        match pump.tick {
            0 => (f64::from(geometry.x) + 8.0, f64::from(geometry.y) + 8.0, Some(true)),
            1 => (f64::from(geometry.x) + 72.0, f64::from(geometry.y) + 56.0, Some(false)),
            2 => (
                f64::from(geometry.x) + f64::from(geometry.width.saturating_sub(1)),
                f64::from(geometry.y) + f64::from(geometry.height.saturating_sub(1)),
                Some(true),
            ),
            3 => (
                f64::from(geometry.x) + f64::from(geometry.width) + 48.0,
                f64::from(geometry.y) + f64::from(geometry.height) + 40.0,
                Some(false),
            ),
            _ => {
                pump.remaining_frames = 0;
                return;
            }
        }
    } else {
        let x_offset: f64 = if pump.tick.is_multiple_of(2) { 24.0 } else { 40.0 };
        let y_offset: f64 = if pump.tick.is_multiple_of(2) { 28.0 } else { 44.0 };
        (
            f64::from(geometry.x) + x_offset.min(f64::from(geometry.width.saturating_sub(1))),
            f64::from(geometry.y) + y_offset.min(f64::from(geometry.height.saturating_sub(1))),
            Some(true),
        )
    };
    pointer.x = x;
    pointer.y = y;

    events.push(BackendInputEvent {
        device: device.to_owned(),
        action: BackendInputAction::PointerMoved { x, y },
    });
    if let Some(pressed) = pressed {
        events.push(BackendInputEvent {
            device: device.to_owned(),
            action: BackendInputAction::PointerButton { button_code: 0x110, pressed },
        });
    }
    wayland_commands.pending_protocol_input_events.extend(events);

    pump.remaining_frames = pump.remaining_frames.saturating_sub(1);
    pump.tick = pump.tick.saturating_add(1);
}

/// Capture the single test window, the single test output, and the current
/// work area after a scenario completes.
fn snapshot_window_output_and_work_area(
    mut app: nekoland_core::prelude::NekolandApp,
) -> (WindowDisplayState, SurfaceGeometry, OutputProperties, WorkArea) {
    let world = app.inner_mut().world_mut();
    let mut window_query =
        world.query::<(&WindowLayout, &WindowMode, &SurfaceGeometry, &XdgWindow)>();
    let windows = window_query
        .iter(world)
        .map(|(layout, mode, geometry, _)| {
            (WindowDisplayState::from_layout_mode(*layout, *mode), geometry.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(windows.len(), 1, "scenario should create exactly one toplevel window");
    let (state, geometry) = windows[0].clone();
    let output = world.query::<&OutputProperties>().iter(world).next().cloned();
    let Some(output) = output else {
        panic!("backend should create one output");
    };
    let Some(work_area) = world.get_resource::<WorkArea>() else {
        panic!("work area should be initialized");
    };
    let work_area = *work_area;
    (state, geometry, output, work_area)
}

/// Convenience wrapper for scenarios that do not need the work-area snapshot.
fn snapshot_window_and_output(
    app: nekoland_core::prelude::NekolandApp,
) -> (WindowDisplayState, SurfaceGeometry, OutputProperties) {
    let (state, geometry, output, _) = snapshot_window_output_and_work_area(app);
    (state, geometry, output)
}

/// Reconstruct the floating-layout centring rule used by the shell systems.
fn centred_position(work_area: &WorkArea, geometry: &SurfaceGeometry) -> (i32, i32) {
    (
        work_area.x + ((work_area.width as i32 - geometry.width as i32) / 2).max(0),
        work_area.y + ((work_area.height as i32 - geometry.height as i32) / 2).max(0),
    )
}

/// Audit every `WindowClosed` message so destroy/close scenarios can assert on
/// the final close stream after the app stops.
fn capture_window_closed_messages(
    mut window_closed: MessageReader<WindowClosed>,
    mut audit: ResMut<ClosedWindowAudit>,
) {
    for event in window_closed.read() {
        audit.surface_ids.push(event.surface_id);
    }
}

/// Automatically issue a compositor-side close request once the scenario's
/// toplevel has been announced.
fn request_server_close_on_window_created(
    mut window_created: MessageReader<WindowCreated>,
    mut auto_close: ResMut<AutoCloseOnCreate>,
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
) {
    if auto_close.issued {
        return;
    }

    let Some(event) = window_created.read().next() else {
        return;
    };

    pending_window_requests.push(WindowServerRequest {
        surface_id: event.surface_id,
        action: WindowServerAction::Close,
    });
    auto_close.issued = true;
}

/// Automatically dismiss the first active popup grab in scenarios that need a
/// server-initiated popup teardown.
fn request_server_popup_dismiss(
    mut auto_dismiss: ResMut<AutoDismissPopup>,
    popups: Query<(&WlSurfaceHandle, &PopupGrab), With<XdgPopup>>,
    mut pending_popup_requests: ResMut<PendingPopupServerRequests>,
) {
    if auto_dismiss.issued {
        return;
    }

    let Some((surface, _)) = popups.iter().find(|(_, grab)| grab.active) else {
        return;
    };

    pending_popup_requests
        .push(PopupServerRequest { surface_id: surface.id, action: PopupServerAction::Dismiss });
    auto_dismiss.issued = true;
}

/// Poll the IPC tree until the scenario exposes a toplevel, then send a close
/// request for that surface id.
fn request_close_over_ipc(socket_path: &std::path::Path) -> Result<u64, common::TestControl> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        let tree = match send_tree_query(socket_path) {
            Ok(tree) => tree,
            Err(error) if ipc_error_is_retryable(&error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(common::TestControl::Fail(format!(
                        "timed out waiting for IPC tree query: {error}"
                    )));
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => {
                return Err(common::TestControl::Fail(error.to_string()));
            }
        };

        let Some(surface_id) = tree.windows.first().map(|window| window.surface_id) else {
            if std::time::Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for IPC tree to expose a toplevel".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        };

        let reply = send_request_to_path(
            socket_path,
            &IpcRequest {
                correlation_id: 2,
                command: IpcCommand::Window(WindowCommand::Close { surface_id }),
            },
        )
        .map_err(|error| {
            if ipc_error_is_skippable(&error) {
                common::TestControl::Skip(error.to_string())
            } else {
                common::TestControl::Fail(error.to_string())
            }
        })?;

        if !reply.ok {
            return Err(common::TestControl::Fail(format!(
                "IPC close request was rejected: {reply:?}"
            )));
        }

        return Ok(surface_id);
    }
}

/// Variant of [`request_close_over_ipc`] that waits until the tree also
/// exposes a popup before closing the parent window.
fn request_close_over_ipc_when_popup_visible(
    socket_path: &std::path::Path,
) -> Result<u64, common::TestControl> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        let tree = match send_tree_query(socket_path) {
            Ok(tree) => tree,
            Err(error) if ipc_error_is_retryable(&error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(common::TestControl::Fail(format!(
                        "timed out waiting for IPC tree query: {error}"
                    )));
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => {
                return Err(common::TestControl::Fail(error.to_string()));
            }
        };

        let Some(surface_id) = tree.windows.first().map(|window| window.surface_id) else {
            if std::time::Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for IPC tree to expose a toplevel".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        };

        if tree.popups.is_empty() {
            if std::time::Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for IPC tree to expose a popup".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        let reply = send_request_to_path(
            socket_path,
            &IpcRequest {
                correlation_id: 4,
                command: IpcCommand::Window(WindowCommand::Close { surface_id }),
            },
        )
        .map_err(|error| {
            if ipc_error_is_skippable(&error) {
                common::TestControl::Skip(error.to_string())
            } else {
                common::TestControl::Fail(error.to_string())
            }
        })?;

        if !reply.ok {
            return Err(common::TestControl::Fail(format!(
                "IPC close request with popup was rejected: {reply:?}"
            )));
        }

        return Ok(surface_id);
    }
}

/// Poll the IPC tree until a popup is visible, then dismiss it over IPC.
fn request_popup_dismiss_over_ipc(
    socket_path: &std::path::Path,
) -> Result<u64, common::TestControl> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        let tree = match send_tree_query(socket_path) {
            Ok(tree) => tree,
            Err(error) if ipc_error_is_retryable(&error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(common::TestControl::Fail(format!(
                        "timed out waiting for IPC tree query: {error}"
                    )));
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => {
                return Err(common::TestControl::Fail(error.to_string()));
            }
        };

        let Some(surface_id) = tree.popups.first().map(|popup| popup.surface_id) else {
            if std::time::Instant::now() >= deadline {
                return Err(common::TestControl::Fail(
                    "timed out waiting for IPC tree to expose a popup".to_owned(),
                ));
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        };

        let reply = send_request_to_path(
            socket_path,
            &IpcRequest {
                correlation_id: 3,
                command: IpcCommand::Popup(PopupCommand::Dismiss { surface_id }),
            },
        )
        .map_err(|error| {
            if ipc_error_is_skippable(&error) {
                common::TestControl::Skip(error.to_string())
            } else {
                common::TestControl::Fail(error.to_string())
            }
        })?;

        if !reply.ok {
            return Err(common::TestControl::Fail(format!(
                "IPC popup dismiss request was rejected: {reply:?}"
            )));
        }

        return Ok(surface_id);
    }
}

/// Subscribe to IPC events and stop once the window-close scenario produced the
/// expected popup dismissal plus the final empty tree snapshot.
fn collect_subscription_events(
    socket_path: &Path,
    subscription: IpcSubscription,
) -> Result<Vec<IpcSubscriptionEvent>, common::TestControl> {
    let mut stream = subscribe_to_path(socket_path, &subscription).map_err(|error| {
        if ipc_error_is_skippable(&error) {
            common::TestControl::Skip(error.to_string())
        } else {
            common::TestControl::Fail(error.to_string())
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut events = Vec::new();

    loop {
        match stream.read_event() {
            Ok(event) => {
                events.push(event);
                if subscription_goal_met(&events) {
                    return Ok(events);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {
                if Instant::now() >= deadline {
                    return Err(common::TestControl::Fail(format!(
                        "timed out waiting for IPC subscription events: {events:?}"
                    )));
                }
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => {
                return Err(common::TestControl::Fail(error.to_string()));
            }
        }
    }
}

/// Return `true` once the subscription stream has observed the specific end
/// state this test cares about.
fn subscription_goal_met(events: &[IpcSubscriptionEvent]) -> bool {
    let saw_popup_dismiss = events
        .iter()
        .any(|event| event.topic == SubscriptionTopic::Popup && event.event == "popup_dismissed");
    let saw_final_tree = events.iter().any(|event| {
        event.topic == SubscriptionTopic::Tree
            && event.event == "tree_changed"
            && event
                .payload
                .clone()
                .and_then(|payload| serde_json::from_value::<TreeSnapshot>(payload).ok())
                .is_some_and(|tree| tree.windows.is_empty() && tree.popups.is_empty())
    });

    saw_popup_dismiss && saw_final_tree
}

/// Retry a single IPC request until the server is ready or the scenario
/// deadline expires.
fn send_request_with_retry(
    socket_path: &std::path::Path,
    request: &IpcRequest,
) -> Result<IpcReply, std::io::Error> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);

    loop {
        match send_request_to_path(socket_path, request) {
            Ok(reply) => return Ok(reply),
            Err(error) if ipc_error_is_retryable(&error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::other(format!(
                        "timed out waiting for IPC request {:?}: {error}",
                        request.command
                    )));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

/// Fetch and decode the current tree snapshot from the IPC control plane.
fn send_tree_query(socket_path: &std::path::Path) -> Result<TreeSnapshot, std::io::Error> {
    let reply = send_request_with_retry(
        socket_path,
        &IpcRequest { correlation_id: 1, command: IpcCommand::Query(QueryCommand::GetTree) },
    )?;

    if !reply.ok {
        return Err(std::io::Error::other(format!("IPC tree query failed: {}", reply.message)));
    }

    let payload = reply.payload.ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidData, "IPC tree query returned no payload")
    })?;

    serde_json::from_value(payload).map_err(std::io::Error::other)
}

/// Classify transient IPC startup errors that the scenario helpers should poll through.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
    )
}

/// Classify environment restrictions that should skip, rather than fail, the test.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}

impl Dispatch<wl_registry::WlRegistry, ()> for ScenarioClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ()));
                    state.maybe_create_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    ));
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

impl Dispatch<wl_seat::WlSeat, ()> for ScenarioClientState {
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ScenarioClientState {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for ScenarioClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let xdg_surface_id = xdg_surface.id();

        if let xdg_surface::Event::Configure { serial, .. } = event {
            if state
                .popup_xdg_surface
                .as_ref()
                .is_some_and(|popup_surface| popup_surface.id() == xdg_surface_id)
            {
                state.popup_configure_serial = Some(serial);
                xdg_surface.ack_configure(serial);
                if let Some(surface) = state.popup_surface.as_ref()
                    && !state.popup_buffer_attached
                {
                    let Some(shm) = state.shm.as_ref() else {
                        panic!("wl_shm should be bound before the popup is configured");
                    };
                    let Ok((file, pool, buffer)) = create_test_buffer(shm, qh) else {
                        panic!("window state client should create a popup wl_shm buffer");
                    };
                    surface.attach(Some(&buffer), 0, 0);
                    state.popup_backing_file = Some(file);
                    state.popup_pool = Some(pool);
                    state.popup_buffer = Some(buffer);
                    state.popup_buffer_attached = true;
                }
                if let Some(surface) = state.popup_surface.as_ref() {
                    surface.commit();
                }
                state.apply_scenario(qh);
                return;
            }

            state.toplevel_configure_count += 1;
            xdg_surface.ack_configure(serial);
            if let Some(surface) = state.base_surface.as_ref() {
                if !state.buffer_attached {
                    let Some(shm) = state.shm.as_ref() else {
                        panic!("wl_shm should be bound before the toplevel is configured");
                    };
                    let Ok((file, pool, buffer)) = create_test_buffer(shm, qh) else {
                        panic!("window state client should create a wl_shm buffer");
                    };
                    surface.attach(Some(&buffer), 0, 0);
                    state.toplevel_backing_file = Some(file);
                    state.toplevel_pool = Some(pool);
                    state.toplevel_buffer = Some(buffer);
                    state.buffer_attached = true;
                }
                surface.commit();
            }

            state.apply_scenario(qh);
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ()> for ScenarioClientState {
    fn event(
        state: &mut Self,
        _popup: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { x, y, width, height } => {
                state.popup_configure_geometry = Some((x, y, width, height));
            }
            xdg_popup::Event::PopupDone => {
                state.received_popup_done = true;

                if (matches!(state.scenario, Some(WindowScenario::PopupGrabInvalidSerial))
                    || matches!(state.scenario, Some(WindowScenario::ServerDismissGrabbedPopup))
                    || matches!(state.scenario, Some(WindowScenario::IpcDismissGrabbedPopup))
                    || matches!(state.scenario, Some(WindowScenario::IpcCloseToplevelWithPopup)))
                    && state.scenario_stage == 2
                {
                    state.destroy_popup_objects();
                    state.scenario_stage = 3;
                    state.final_request_sent = true;
                } else if matches!(state.scenario, Some(WindowScenario::WorkspaceVisibility))
                    && state.scenario_stage == 2
                {
                    state.destroy_popup_objects();
                    if let Err(error) = state.switch_workspace("1") {
                        state.terminal_error = Some(error);
                        return;
                    }

                    state.scenario_stage = 3;
                    state.final_request_sent = true;
                }
            }
            xdg_popup::Event::Repositioned { token } => {
                state.popup_repositioned_token = Some(token);
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for ScenarioClientState {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.received_toplevel_close = true;

            if (matches!(state.scenario, Some(WindowScenario::ServerCloseToplevel))
                || matches!(state.scenario, Some(WindowScenario::IpcCloseToplevel))
                || matches!(state.scenario, Some(WindowScenario::IpcCloseToplevelWithPopup)))
                && !state.final_request_sent
            {
                if let Some(toplevel) = state.toplevel.take() {
                    toplevel.destroy();
                }
                if let Some(xdg_surface) = state.toplevel_xdg_surface.take() {
                    xdg_surface.destroy();
                }
                if let Some(surface) = state.base_surface.take() {
                    surface.destroy();
                }
                state.final_request_sent = true;
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for ScenarioClientState {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            state.latest_pointer_button_serial = Some(serial);
            state.apply_scenario(qh);
        }
    }
}

delegate_noop!(ScenarioClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(ScenarioClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(ScenarioClientState: ignore wl_surface::WlSurface);
delegate_noop!(ScenarioClientState: ignore wl_shm::WlShm);
delegate_noop!(ScenarioClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ScenarioClientState: ignore xdg_positioner::XdgPositioner);

impl ScenarioClientState {
    /// Create the scenario's root toplevel once both compositor globals are bound.
    fn maybe_create_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_some() || self.compositor.is_none() || self.wm_base.is_none() {
            return;
        }

        let (Some(compositor), Some(wm_base)) = (self.compositor.as_ref(), self.wm_base.as_ref())
        else {
            panic!("compositor and wm_base should be present before creating the toplevel");
        };

        let base_surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&base_surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        base_surface.commit();

        self.base_surface = Some(base_surface);
        self.toplevel_xdg_surface = Some(xdg_surface);
        self.toplevel = Some(toplevel);
    }

    /// Advance the scenario state machine in response to protocol callbacks or
    /// fresh input serials.
    fn apply_scenario(&mut self, qh: &QueueHandle<Self>) {
        let Some(scenario) = self.scenario else {
            return;
        };

        let Some(base_surface) = self.base_surface.as_ref() else {
            panic!("scenario requires an existing base surface");
        };
        let Some(toplevel) = self.toplevel.as_ref() else {
            panic!("scenario requires a toplevel object");
        };

        match scenario {
            WindowScenario::Maximize if self.scenario_stage == 0 => {
                toplevel.set_maximized();
                base_surface.commit();
                self.scenario_stage = 1;
            }
            WindowScenario::RestoreMaximize if self.scenario_stage == 0 => {
                toplevel.set_maximized();
                base_surface.commit();
                self.scenario_stage = 1;
            }
            WindowScenario::RestoreMaximize
                if self.scenario_stage == 1 && self.toplevel_configure_count >= 2 =>
            {
                toplevel.unset_maximized();
                base_surface.commit();
                self.scenario_stage = 2;
                self.final_request_sent = true;
            }
            WindowScenario::FullscreenPopup if self.scenario_stage == 0 => {
                toplevel.set_fullscreen(None);
                base_surface.commit();
                self.create_popup(qh, None);
                self.scenario_stage = 1;
            }
            WindowScenario::RestoreFullscreen if self.scenario_stage == 0 => {
                toplevel.set_fullscreen(None);
                base_surface.commit();
                self.scenario_stage = 1;
            }
            WindowScenario::RestoreFullscreen
                if self.scenario_stage == 1 && self.toplevel_configure_count >= 2 =>
            {
                toplevel.unset_fullscreen();
                base_surface.commit();
                self.scenario_stage = 2;
                self.final_request_sent = true;
            }
            WindowScenario::Minimize if self.scenario_stage == 0 => {
                toplevel.set_minimized();
                self.scenario_stage = 1;
                self.final_request_sent = true;
            }
            WindowScenario::MoveResize
                if self.scenario_stage == 0 && self.latest_pointer_button_serial.is_some() =>
            {
                let Some(serial) = self.latest_pointer_button_serial else {
                    panic!("move+resize scenario requires a real wl_pointer button serial");
                };
                let Some(seat) = self.seat.as_ref() else {
                    panic!("move+resize scenario requires wl_seat");
                };
                toplevel._move(seat, serial);
                self.interactive_request_serial = Some(serial);
                self.scenario_stage = 1;
                self.final_request_sent = true;
            }
            WindowScenario::MoveResizeInvalidSerial if self.scenario_stage == 0 => {
                let Some(seat) = self.seat.as_ref() else {
                    panic!("move+resize scenario requires wl_seat");
                };
                toplevel._move(seat, 41);
                toplevel.resize(seat, 42, xdg_toplevel::ResizeEdge::BottomRight);
                self.scenario_stage = 1;
                self.final_request_sent = true;
            }
            WindowScenario::PopupGrab
                if self.scenario_stage == 0 && self.latest_pointer_button_serial.is_some() =>
            {
                let Some(serial) = self.latest_pointer_button_serial else {
                    panic!("popup grab scenario requires a real wl_pointer button serial");
                };
                self.create_popup(qh, Some(serial));
                self.interactive_request_serial = Some(serial);
                self.scenario_stage = 1;
                self.final_request_sent = true;
            }
            WindowScenario::ServerDismissGrabbedPopup | WindowScenario::IpcDismissGrabbedPopup
                if self.scenario_stage == 0 && self.latest_pointer_button_serial.is_some() =>
            {
                let Some(serial) = self.latest_pointer_button_serial else {
                    panic!("popup dismiss scenario requires a real wl_pointer button serial");
                };
                self.create_popup(qh, Some(serial));
                self.interactive_request_serial = Some(serial);
                self.scenario_stage = 1;
                self.final_request_sent = true;
            }
            WindowScenario::PopupGrabInvalidSerial if self.scenario_stage == 0 => {
                self.create_popup(qh, Some(77));
                self.scenario_stage = 2;
                self.final_request_sent = true;
            }
            WindowScenario::PopupReposition if self.scenario_stage == 0 => {
                self.create_popup(qh, None);
                self.scenario_stage = 1;
            }
            WindowScenario::PopupReposition
                if self.scenario_stage == 1 && self.popup_configure_serial.is_some() =>
            {
                let Some(popup) = self.popup.as_ref() else {
                    panic!("popup reposition scenario requires xdg_popup");
                };
                let positioner = self.make_positioner(
                    qh,
                    PopupPositionerSpec {
                        width: 300,
                        height: 140,
                        anchor_x: 80,
                        anchor_y: 48,
                        anchor_width: 96,
                        anchor_height: 40,
                        offset_x: 20,
                        offset_y: 16,
                    },
                );
                popup.reposition(&positioner, 91);
                self.scenario_stage = 2;
                self.final_request_sent = true;
            }
            WindowScenario::PopupDestroy if self.scenario_stage == 0 => {
                self.create_popup(qh, None);
                self.scenario_stage = 1;
            }
            WindowScenario::PopupDestroy
                if self.scenario_stage == 1 && self.popup_configure_serial.is_some() =>
            {
                let Some(popup) = self.popup.as_ref() else {
                    panic!("popup destroy scenario requires xdg_popup");
                };
                popup.destroy();
                self.scenario_stage = 2;
                self.final_request_sent = true;
            }
            WindowScenario::WorkspaceVisibility if self.scenario_stage == 0 => {
                self.create_popup(qh, None);
                self.scenario_stage = 1;
            }
            WindowScenario::IpcCloseToplevelWithPopup if self.scenario_stage == 0 => {
                self.create_popup(qh, None);
                self.scenario_stage = 1;
            }
            WindowScenario::WorkspaceVisibility
                if self.scenario_stage == 1 && self.popup_configure_serial.is_some() =>
            {
                if let Err(error) = self.switch_workspace("2") {
                    self.terminal_error = Some(error);
                    return;
                }

                self.scenario_stage = 2;
            }
            WindowScenario::ToplevelDestroy if self.scenario_stage == 0 => {
                let Some(toplevel) = self.toplevel.as_ref() else {
                    panic!("toplevel destroy scenario requires xdg_toplevel");
                };
                let Some(xdg_surface) = self.toplevel_xdg_surface.as_ref() else {
                    panic!("toplevel destroy scenario requires xdg_surface");
                };
                toplevel.destroy();
                xdg_surface.destroy();
                base_surface.destroy();
                self.scenario_stage = 1;
                self.final_request_sent = true;
            }
            WindowScenario::ServerCloseToplevel | WindowScenario::IpcCloseToplevel => {}
            _ => {}
        }
    }

    /// Create the scenario popup and optionally request an explicit popup grab.
    fn create_popup(&mut self, qh: &QueueHandle<Self>, grab_serial: Option<u32>) {
        if self.popup_surface.is_some() {
            return;
        }

        let Some(compositor) = self.compositor.as_ref() else {
            panic!("popup creation requires a compositor global");
        };
        let Some(wm_base) = self.wm_base.as_ref() else {
            panic!("popup creation requires xdg_wm_base");
        };
        let Some(parent) = self.toplevel_xdg_surface.as_ref() else {
            panic!("popup creation requires a parent xdg_surface");
        };

        let popup_surface = compositor.create_surface(qh, ());
        let popup_xdg_surface = wm_base.get_xdg_surface(&popup_surface, qh, ());
        let positioner = self.make_positioner(
            qh,
            PopupPositionerSpec {
                width: 240,
                height: 120,
                anchor_x: 24,
                anchor_y: 24,
                anchor_width: 64,
                anchor_height: 32,
                offset_x: 16,
                offset_y: 12,
            },
        );
        let popup = popup_xdg_surface.get_popup(Some(parent), &positioner, qh, ());
        if let Some(serial) = grab_serial {
            let Some(seat) = self.seat.as_ref() else {
                panic!("popup grab scenarios require wl_seat");
            };
            popup.grab(seat, serial);
        }
        popup_surface.commit();

        self.popup_surface = Some(popup_surface);
        self.popup_xdg_surface = Some(popup_xdg_surface);
        self.popup = Some(popup);
    }

    /// Destroy every popup-side Wayland object and reset the popup bookkeeping fields.
    fn destroy_popup_objects(&mut self) {
        if let Some(popup) = self.popup.take() {
            popup.destroy();
        }
        if let Some(xdg_surface) = self.popup_xdg_surface.take() {
            xdg_surface.destroy();
        }
        if let Some(surface) = self.popup_surface.take() {
            surface.destroy();
        }
        self.popup_pool = None;
        self.popup_buffer = None;
        self.popup_backing_file = None;
        self.popup_buffer_attached = false;
    }

    /// Ask the compositor to switch workspaces through IPC from inside the client thread.
    fn switch_workspace(&self, workspace: &str) -> Result<(), String> {
        let Some(socket_path) = self.ipc_socket_path.as_deref() else {
            return Err("workspace switch scenario requires an IPC socket path".to_owned());
        };

        send_request_with_retry(
            socket_path,
            &IpcRequest {
                correlation_id: 100 + u64::from(self.scenario_stage),
                command: IpcCommand::Workspace(WorkspaceCommand::Switch {
                    workspace: workspace.to_owned(),
                }),
            },
        )
        .map(|_| ())
        .map_err(|error| format!("workspace switch to {workspace} failed: {error}"))
    }

    /// Create a reusable XDG positioner for popup creation or reposition requests.
    fn make_positioner(
        &self,
        qh: &QueueHandle<Self>,
        spec: PopupPositionerSpec,
    ) -> xdg_positioner::XdgPositioner {
        let Some(wm_base) = self.wm_base.as_ref() else {
            panic!("positioner creation requires xdg_wm_base");
        };
        let PopupPositionerSpec {
            width,
            height,
            anchor_x,
            anchor_y,
            anchor_width,
            anchor_height,
            offset_x,
            offset_y,
        } = spec;
        let positioner = wm_base.create_positioner(qh, ());
        positioner.set_size(width, height);
        positioner.set_anchor_rect(anchor_x, anchor_y, anchor_width, anchor_height);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_offset(offset_x, offset_y);
        positioner.set_reactive();
        positioner
    }

    /// Return whether the current scenario has observed enough protocol state
    /// to stop the helper client loop.
    fn is_complete(&self) -> bool {
        match self.scenario {
            Some(WindowScenario::Maximize) => {
                self.scenario_stage >= 1 && self.toplevel_configure_count >= 2
            }
            Some(WindowScenario::RestoreMaximize) => self.final_request_sent,
            Some(WindowScenario::FullscreenPopup) => {
                self.scenario_stage >= 1
                    && self.toplevel_configure_count >= 2
                    && self.popup_configure_serial.is_some()
            }
            Some(WindowScenario::RestoreFullscreen) => self.final_request_sent,
            Some(WindowScenario::Minimize) => self.final_request_sent,
            Some(WindowScenario::MoveResize) => self.final_request_sent,
            Some(WindowScenario::MoveResizeInvalidSerial) => self.final_request_sent,
            Some(WindowScenario::PopupGrab) => {
                self.final_request_sent && self.popup_configure_serial.is_some()
            }
            Some(WindowScenario::ServerDismissGrabbedPopup) => {
                self.received_popup_done && self.final_request_sent
            }
            Some(WindowScenario::IpcDismissGrabbedPopup) => {
                self.received_popup_done && self.final_request_sent
            }
            Some(WindowScenario::PopupGrabInvalidSerial) => {
                self.final_request_sent && self.received_popup_done
            }
            Some(WindowScenario::PopupReposition) => {
                self.final_request_sent && self.popup_repositioned_token == Some(91)
            }
            Some(WindowScenario::PopupDestroy) => self.final_request_sent,
            Some(WindowScenario::ToplevelDestroy) => self.final_request_sent,
            Some(WindowScenario::ServerCloseToplevel) => {
                self.received_toplevel_close && self.final_request_sent
            }
            Some(WindowScenario::IpcCloseToplevel) => {
                self.received_toplevel_close && self.final_request_sent
            }
            Some(WindowScenario::IpcCloseToplevelWithPopup) => {
                self.received_toplevel_close && self.received_popup_done && self.final_request_sent
            }
            Some(WindowScenario::WorkspaceVisibility) => {
                self.received_popup_done
                    && self.final_request_sent
                    && self.toplevel_configure_count >= 2
            }
            None => false,
        }
    }
}

/// Create a small shared-memory buffer so the test client can commit real
/// toplevel and popup contents.
fn create_test_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<ScenarioClientState>,
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
