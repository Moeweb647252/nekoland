//! Shell plugin wiring and shell-owned boundary synchronization.

use bevy_app::App;
use bevy_ecs::prelude::{Res, ResMut};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{LayoutSchedule, PostRenderSchedule};
use nekoland_ecs::events::{
    ExternalCommandFailed, ExternalCommandLaunched, WindowClosed, WindowCreated, WindowMoved,
};
use nekoland_ecs::resources::register_entity_index_hooks;
use nekoland_ecs::resources::{
    CommandHistoryState, FocusedOutputState, GlobalPointerPosition, OutputOverlayState,
    OverlayUiFrame, PendingExternalCommandRequests, PendingOutputControls,
    PendingOutputOverlayControls, PendingOutputServerRequests, PendingPopupServerRequests,
    PendingWindowControls, PendingWindowServerRequests, PendingWorkspaceControls, ShellRenderInput,
    SurfacePresentationSnapshot, WaylandCommands, WaylandFeedback, WaylandIngress,
    WindowStackingState, WorkArea, WorkspaceTilingState,
};

use crate::{
    commands, decorations, focus,
    interaction::{self, ActiveWindowGrab},
    layer, layout, presentation, surface_presentation, viewport, window_control, window_lifecycle,
    window_switcher, workspace, xdg,
};

#[derive(Debug, Default, Clone, Copy)]
/// Main-world plugin that owns shell policy and synchronizes shell-facing boundary resources.
pub struct ShellPlugin;

impl NekolandPlugin for ShellPlugin {
    /// Register shell resources and the two-phase layout pipeline that first
    /// mutates topology, then derives geometry/focus/decorations.
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingPopupServerRequests>()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingWorkspaceControls>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<WorkArea>()
            .init_resource::<ActiveWindowGrab>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .init_resource::<OutputOverlayState>()
            .init_resource::<OverlayUiFrame>()
            .init_resource::<SurfacePresentationSnapshot>()
            .init_resource::<WaylandCommands>()
            .init_resource::<WaylandIngress>()
            .init_resource::<WaylandFeedback>()
            .init_resource::<ShellRenderInput>()
            .init_resource::<FocusedOutputState>()
            .init_resource::<layer::DeferredLayerRequests>()
            .init_resource::<xdg::DeferredXdgRequests>()
            .init_resource::<xdg::popup::DeferredPopupEvents>()
            .init_resource::<window_lifecycle::DeferredWindowEvents>()
            .init_resource::<workspace::RememberedOutputWorkspaceState>()
            .init_resource::<window_switcher::WindowSwitcherState>()
            .init_resource::<CommandHistoryState>()
            .init_resource::<commands::StartupActionState>()
            .init_resource::<PendingExternalCommandRequests>();

        register_entity_index_hooks(app.world_mut());

        app.add_message::<WindowCreated>()
            .add_message::<WindowClosed>()
            .add_message::<WindowMoved>()
            .add_message::<ExternalCommandLaunched>()
            .add_message::<ExternalCommandFailed>()
            .add_systems(
                LayoutSchedule,
                // The first chain mutates topology and pending requests; the second consumes that
                // normalized state to derive geometry, focus, and server-side decorations.
                (
                    (
                        commands::quit_shortcut_system,
                        commands::startup_action_queue_system,
                        commands::external_command_launch_system,
                        commands::command_history_system,
                        (
                            workspace::workspace_switch_system,
                            workspace::workspace_command_system,
                            workspace::output_workspace_housekeeping_system,
                            workspace::remember_output_workspace_routes_system,
                            workspace::sync_active_workspace_marker_system,
                            workspace::sync_workspace_disabled_state_system,
                        )
                            .chain()
                            .run_if(workspace::workspace_reconciliation_needed),
                        layer::arrange::layer_lifecycle_system,
                        layer::arrange::sync_layer_output_relationships_system.run_if(
                            layer::arrange::layer_output_relationship_reconciliation_needed,
                        ),
                        window_lifecycle::window_lifecycle_system,
                        xdg::popup::popup_management_system,
                        xdg::configure::configure_sequence_system,
                        window_switcher::window_switcher_input_system,
                        window_control::window_control_request_system,
                    )
                        .chain(),
                    (
                        layer::arrange::layer_arrangement_system,
                        layer::arrange::work_area_system,
                        layout::tiling::tiling_layout_system,
                        layout::floating::floating_layout_system,
                        layout::fullscreen::fullscreen_layout_system,
                        viewport::window_viewport_projection_system,
                        xdg::popup::popup_projection_system,
                        window_switcher::window_switcher_overlay_system,
                        focus::pointer_button_focus_system,
                        interaction::window_grab_system,
                        layout::stacking::stacking_layout_system,
                        surface_presentation::surface_presentation_snapshot_system,
                        presentation::window_presentation_sync_system,
                        focus::focus_management_system,
                        decorations::server_decoration_system,
                        sync_shell_render_boundary_system,
                        sync_wayland_commands_boundary_system,
                    )
                        .chain(),
                )
                    .chain(),
            )
            .add_systems(PostRenderSchedule, clear_overlay_ui_frame_system);
    }
}

/// Mirrors shell-owned protocol/backend requests into the one-way `WaylandCommands` boundary.
fn sync_wayland_commands_boundary_system(
    pending_output_controls: Res<'_, PendingOutputControls>,
    pending_output_overlay_controls: Res<'_, PendingOutputOverlayControls>,
    pending_output_server_requests: Res<'_, PendingOutputServerRequests>,
    mut pending_window_server_requests: ResMut<'_, PendingWindowServerRequests>,
    pending_popup_server_requests: Res<'_, PendingPopupServerRequests>,
    mut wayland_commands: ResMut<'_, WaylandCommands>,
) {
    let pending_protocol_input_events = wayland_commands.pending_protocol_input_events.clone();
    let pending_window_server_requests_boundary = pending_window_server_requests.clone();
    pending_window_server_requests.clear();
    *wayland_commands = WaylandCommands {
        pending_output_controls: pending_output_controls.clone(),
        pending_output_overlay_controls: pending_output_overlay_controls.clone(),
        pending_output_server_requests: pending_output_server_requests.clone(),
        pending_window_server_requests: pending_window_server_requests_boundary,
        pending_popup_server_requests: pending_popup_server_requests.clone(),
        pending_protocol_input_events,
    };
}

/// Mirrors shell-owned presentation state into the render-facing boundary snapshot.
fn sync_shell_render_boundary_system(
    pointer: Res<'_, GlobalPointerPosition>,
    wayland_ingress: Res<'_, WaylandIngress>,
    wayland_feedback: Res<'_, WaylandFeedback>,
    surface_presentation: Res<'_, SurfacePresentationSnapshot>,
    output_overlays: Res<'_, OutputOverlayState>,
    overlay_ui: Res<'_, OverlayUiFrame>,
    mut shell_render_input: ResMut<'_, ShellRenderInput>,
) {
    *shell_render_input = ShellRenderInput {
        pointer: pointer.clone(),
        cursor_image: wayland_ingress.cursor_image.clone(),
        surface_presentation: surface_presentation.clone(),
        output_overlays: output_overlays.clone(),
        overlay_ui: overlay_ui.clone(),
        pending_screenshot_requests: wayland_feedback.pending_screenshot_requests.clone(),
    };
}

fn clear_overlay_ui_frame_system(mut overlay_ui: ResMut<'_, OverlayUiFrame>) {
    overlay_ui.clear();
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::components::{
        BufferState, WindowLayout, WindowMode, WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::events::{PointerButton, WindowMoved};
    use nekoland_ecs::resources::{
        CursorImageSnapshot, EntityIndex, GlobalPointerPosition, KeyboardFocusState,
        OutputOverlayState, OverlayUiFrame, OverlayUiLayer, PendingOutputControls,
        PendingOutputOverlayControls, PendingOutputServerRequests, PendingPopupServerRequests,
        PendingScreenshotRequests, PendingWindowServerRequests, RenderColor, RenderRect,
        ShellRenderInput, SurfaceExtent, SurfacePresentationSnapshot, WaylandCommands,
        WaylandFeedback, WaylandIngress, WindowServerAction, WindowServerRequest,
        WindowStackingState, WorkArea, register_entity_index_hooks,
    };

    use crate::interaction::{
        ActiveWindowGrab, WindowGrabMode, begin_window_grab, window_grab_system,
    };
    use crate::presentation::window_presentation_sync_system;

    use super::{
        clear_overlay_ui_frame_system, sync_shell_render_boundary_system,
        sync_wayland_commands_boundary_system,
    };

    #[test]
    fn shell_render_boundary_captures_shell_owned_snapshots() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 12.0, y: 34.0 });
        world.insert_resource(WaylandIngress {
            cursor_image: CursorImageSnapshot::Named { icon_name: "default".to_owned() },
            ..WaylandIngress::default()
        });
        world.insert_resource(SurfacePresentationSnapshot::default());
        world.insert_resource(OutputOverlayState::default());
        world.insert_resource(OverlayUiFrame::default());
        let mut pending_screenshot_requests = PendingScreenshotRequests::default();
        let _ = pending_screenshot_requests.request_output(OutputId(7));
        world.insert_resource(WaylandFeedback {
            pending_screenshot_requests,
            ..WaylandFeedback::default()
        });
        world.init_resource::<ShellRenderInput>();

        let mut system = IntoSystem::into_system(sync_shell_render_boundary_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let boundary = world.resource::<ShellRenderInput>();
        assert_eq!(boundary.pointer.x, 12.0);
        assert_eq!(boundary.pointer.y, 34.0);
        assert_eq!(
            boundary.cursor_image,
            CursorImageSnapshot::Named { icon_name: "default".to_owned() }
        );
        assert_eq!(boundary.pending_screenshot_requests.requests.len(), 1);
        assert_eq!(boundary.pending_screenshot_requests.requests[0].output_id, OutputId(7));
    }

    #[test]
    fn overlay_ui_frame_is_cleared_after_boundary_sync() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition::default());
        world.insert_resource(WaylandIngress::default());
        world.insert_resource(SurfacePresentationSnapshot::default());
        world.insert_resource(OutputOverlayState::default());
        world.insert_resource(WaylandFeedback::default());
        world.init_resource::<ShellRenderInput>();

        let mut overlay_ui = OverlayUiFrame::default();
        overlay_ui.output(OutputId(9)).panel(
            "hud.panel",
            OverlayUiLayer::Main,
            RenderRect { x: 10, y: 20, width: 100, height: 40 },
            None,
            RenderColor { r: 1, g: 2, b: 3, a: 255 },
            1.0,
            5,
        );
        world.insert_resource(overlay_ui);

        let mut sync_system = IntoSystem::into_system(sync_shell_render_boundary_system);
        sync_system.initialize(&mut world);
        let _ = sync_system.run((), &mut world);

        let mut clear_system = IntoSystem::into_system(clear_overlay_ui_frame_system);
        clear_system.initialize(&mut world);
        let _ = clear_system.run((), &mut world);

        assert!(world.resource::<OverlayUiFrame>().outputs.is_empty());
        let boundary = world.resource::<ShellRenderInput>();
        let output_frame = boundary
            .overlay_ui
            .outputs
            .get(&OutputId(9))
            .expect("shell render boundary should keep this frame's overlay UI");
        assert_eq!(output_frame.primitives.len(), 1);
    }

    #[test]
    fn wayland_commands_boundary_drains_one_shot_server_requests() {
        let mut world = World::default();
        let mut pending_window_server_requests = PendingWindowServerRequests::default();
        pending_window_server_requests.push(WindowServerRequest {
            surface_id: 7,
            action: WindowServerAction::SyncXdgToplevelState {
                size: Some(SurfaceExtent { width: 800, height: 600 }),
                fullscreen: false,
                maximized: false,
                resizing: false,
            },
        });
        world.insert_resource(PendingOutputControls::default());
        world.insert_resource(PendingOutputOverlayControls::default());
        world.insert_resource(PendingOutputServerRequests::default());
        world.insert_resource(pending_window_server_requests);
        world.insert_resource(PendingPopupServerRequests::default());
        world.insert_resource(WaylandCommands::default());

        let mut system = IntoSystem::into_system(sync_wayland_commands_boundary_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        assert!(world.resource::<PendingWindowServerRequests>().is_empty());
        assert_eq!(world.resource::<WaylandCommands>().pending_window_server_requests.len(), 1);
    }

    #[test]
    fn native_xdg_resize_flows_into_wayland_commands_boundary() {
        let mut app = NekolandApp::new("native-xdg-resize-boundary-test");
        register_entity_index_hooks(app.inner_mut().world_mut());
        app.inner_mut()
            .init_resource::<EntityIndex>()
            .init_resource::<WaylandIngress>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<WaylandCommands>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkArea>()
            .insert_resource(GlobalPointerPosition { x: 100.0, y: 100.0 })
            .insert_resource(ActiveWindowGrab::default())
            .add_message::<PointerButton>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (
                    nekoland_ecs::resources::rebuild_entity_index_system,
                    window_grab_system,
                    window_presentation_sync_system,
                    sync_wayland_commands_boundary_system,
                )
                    .chain(),
            );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 99 },
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                },
                scene_geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                buffer: BufferState { attached: true, scale: 1 },
                window: XdgWindow::default(),
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            })
            .id();

        {
            let world = app.inner_mut().world_mut();
            let geometry = world
                .get::<WindowSceneGeometry>(entity)
                .cloned()
                .expect("test window should expose scene geometry");
            let pointer = world.resource::<GlobalPointerPosition>().clone();
            begin_window_grab(
                &mut world.resource_mut::<ActiveWindowGrab>(),
                99,
                WindowGrabMode::Resize { edges: nekoland_ecs::resources::ResizeEdges::BottomRight },
                &pointer,
                &geometry,
            );
        }
        app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>().x = 140.0;
        app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>().y = 140.0;

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let requests = app
            .inner()
            .world()
            .resource::<WaylandCommands>()
            .pending_window_server_requests
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].surface_id, 99);
        assert_eq!(
            requests[0].action,
            WindowServerAction::SyncPresentation {
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: 10,
                    y: 20,
                    width: 840,
                    height: 640,
                },
                scene_geometry: Some(WindowSceneGeometry { x: 10, y: 20, width: 840, height: 640 }),
                fullscreen: false,
                maximized: false,
                resizing: true,
            }
        );
    }
}
