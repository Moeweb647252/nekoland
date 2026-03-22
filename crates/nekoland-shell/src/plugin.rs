use bevy_app::App;
use bevy_ecs::prelude::{Res, ResMut};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::events::{
    ExternalCommandFailed, ExternalCommandLaunched, WindowClosed, WindowCreated, WindowMoved,
};
use nekoland_ecs::resources::register_entity_index_hooks;
use nekoland_ecs::resources::{
    CommandHistoryState, GlobalPointerPosition, OutputOverlayState, PendingExternalCommandRequests,
    PendingOutputControls, PendingOutputOverlayControls, PendingOutputServerRequests,
    PendingPopupServerRequests, PendingWindowControls, PendingWindowServerRequests,
    PendingWorkspaceControls, PrimaryOutputState, ShellRenderInput, SurfacePresentationSnapshot,
    WaylandCommands, WaylandFeedback, WaylandIngress, WindowStackingState, WorkArea,
    WorkspaceTilingState,
};

use crate::{
    commands, decorations, focus,
    interaction::{self, ActiveWindowGrab},
    layer, layout, presentation, surface_presentation, viewport, window_control, workspace, x11,
    xdg,
};

#[derive(Debug, Default, Clone, Copy)]
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
            .init_resource::<SurfacePresentationSnapshot>()
            .init_resource::<WaylandCommands>()
            .init_resource::<ShellRenderInput>()
            .init_resource::<workspace::RememberedOutputWorkspaceState>()
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
                        sync_shell_inputs_from_wayland_ingress_system,
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
                        xdg::toplevel::toplevel_lifecycle_system,
                        xdg::popup::popup_management_system,
                        xdg::configure::configure_sequence_system,
                        window_control::window_control_request_system,
                    )
                        .chain(),
                    (
                        layer::arrange::layer_arrangement_system,
                        layer::arrange::work_area_system,
                        x11::xwayland::xwayland_bridge_system,
                        layout::tiling::tiling_layout_system,
                        layout::floating::floating_layout_system,
                        layout::fullscreen::fullscreen_layout_system,
                        viewport::window_viewport_projection_system,
                        xdg::popup::popup_projection_system,
                        focus::pointer_button_focus_system,
                        interaction::window_grab_system,
                        layout::stacking::stacking_layout_system,
                        surface_presentation::surface_presentation_snapshot_system,
                        presentation::window_presentation_sync_system,
                        focus::focus_management_system,
                        decorations::server_decoration_system,
                        sync_shell_render_mailbox_system,
                        sync_wayland_commands_mailbox_system,
                    )
                        .chain(),
                )
                    .chain(),
            );
    }
}

fn sync_shell_inputs_from_wayland_ingress_system(
    wayland_ingress: Option<Res<'_, WaylandIngress>>,
    primary_output: Option<ResMut<'_, PrimaryOutputState>>,
) {
    let Some(wayland_ingress) = wayland_ingress else {
        return;
    };
    let Some(mut primary_output) = primary_output else {
        return;
    };

    *primary_output = wayland_ingress.primary_output.clone();
}

fn sync_wayland_commands_mailbox_system(
    pending_output_controls: Res<'_, PendingOutputControls>,
    pending_output_overlay_controls: Res<'_, PendingOutputOverlayControls>,
    pending_output_server_requests: Res<'_, PendingOutputServerRequests>,
    pending_window_server_requests: Res<'_, PendingWindowServerRequests>,
    pending_popup_server_requests: Res<'_, PendingPopupServerRequests>,
    mut wayland_commands: ResMut<'_, WaylandCommands>,
) {
    let pending_protocol_input_events = wayland_commands.pending_protocol_input_events.clone();
    *wayland_commands = WaylandCommands {
        pending_output_controls: pending_output_controls.clone(),
        pending_output_overlay_controls: pending_output_overlay_controls.clone(),
        pending_output_server_requests: pending_output_server_requests.clone(),
        pending_window_server_requests: pending_window_server_requests.clone(),
        pending_popup_server_requests: pending_popup_server_requests.clone(),
        pending_protocol_input_events,
    };
}

fn sync_shell_render_mailbox_system(
    pointer: Res<'_, GlobalPointerPosition>,
    wayland_ingress: Option<Res<'_, WaylandIngress>>,
    wayland_feedback: Option<Res<'_, WaylandFeedback>>,
    surface_presentation: Res<'_, SurfacePresentationSnapshot>,
    output_overlays: Res<'_, OutputOverlayState>,
    mut shell_render_input: ResMut<'_, ShellRenderInput>,
) {
    *shell_render_input = ShellRenderInput {
        pointer: pointer.clone(),
        cursor_image: wayland_ingress
            .map(|wayland_ingress| wayland_ingress.cursor_image.clone())
            .unwrap_or_default(),
        surface_presentation: surface_presentation.clone(),
        output_overlays: output_overlays.clone(),
        pending_screenshot_requests: wayland_feedback
            .map(|wayland_feedback| wayland_feedback.pending_screenshot_requests.clone())
            .unwrap_or_default(),
    };
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::prelude::SurfaceId;
    use nekoland_ecs::resources::{
        CursorImageSnapshot, GlobalPointerPosition, OutputOverlayState, PendingLayerRequests,
        PendingScreenshotRequests, PendingWindowControls, PendingX11Requests, PendingXdgRequests,
        PrimaryOutputState, ShellRenderInput, SurfacePresentationSnapshot, WaylandFeedback,
        WaylandIngress,
    };

    use super::{sync_shell_inputs_from_wayland_ingress_system, sync_shell_render_mailbox_system};

    #[test]
    fn shell_inputs_sync_primary_output_and_protocol_requests_from_wayland_ingress() {
        let mut world = World::default();
        world.insert_resource(WaylandIngress {
            primary_output: PrimaryOutputState { id: Some(OutputId(9)) },
            pending_window_controls: {
                let mut controls = PendingWindowControls::default();
                controls.surface(SurfaceId(42)).focus();
                controls
            },
            ..WaylandIngress::default()
        });
        world.init_resource::<PrimaryOutputState>();
        world.init_resource::<PendingWindowControls>();

        let mut system = IntoSystem::into_system(sync_shell_inputs_from_wayland_ingress_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        assert_eq!(world.resource::<PrimaryOutputState>().id, Some(OutputId(9)));
        assert!(world.resource::<PendingWindowControls>().is_empty());
        assert!(world.get_resource::<PendingXdgRequests>().is_none());
        assert!(world.get_resource::<PendingX11Requests>().is_none());
        assert!(world.get_resource::<PendingLayerRequests>().is_none());
    }

    #[test]
    fn shell_render_mailbox_captures_shell_owned_snapshots() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 12.0, y: 34.0 });
        world.insert_resource(WaylandIngress {
            cursor_image: CursorImageSnapshot::Named { icon_name: "default".to_owned() },
            ..WaylandIngress::default()
        });
        world.insert_resource(SurfacePresentationSnapshot::default());
        world.insert_resource(OutputOverlayState::default());
        let mut pending_screenshot_requests = PendingScreenshotRequests::default();
        let _ = pending_screenshot_requests.request_output(OutputId(7));
        world.insert_resource(WaylandFeedback {
            pending_screenshot_requests,
            ..WaylandFeedback::default()
        });
        world.init_resource::<ShellRenderInput>();

        let mut system = IntoSystem::into_system(sync_shell_render_mailbox_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let mailbox = world.resource::<ShellRenderInput>();
        assert_eq!(mailbox.pointer.x, 12.0);
        assert_eq!(mailbox.pointer.y, 34.0);
        assert_eq!(
            mailbox.cursor_image,
            CursorImageSnapshot::Named { icon_name: "default".to_owned() }
        );
        assert_eq!(mailbox.pending_screenshot_requests.requests.len(), 1);
        assert_eq!(mailbox.pending_screenshot_requests.requests[0].output_id, OutputId(7));
    }
}
