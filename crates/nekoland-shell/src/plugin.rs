use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::events::{
    ExternalCommandFailed, ExternalCommandLaunched, WindowClosed, WindowCreated, WindowMoved,
};
use nekoland_ecs::resources::{
    CommandHistoryState, PendingExternalCommandRequests, PendingLayerRequests,
    PendingPopupServerRequests, PendingWindowControls, PendingWindowServerRequests,
    PendingWorkspaceControls, PendingX11Requests, PendingXdgRequests, SurfacePresentationSnapshot,
    WindowStackingState, WorkArea, WorkspaceTilingState,
};
use nekoland_ecs::resources::{EntityIndex, rebuild_entity_index_system};

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
        app.init_resource::<PendingXdgRequests>()
            .init_resource::<PendingX11Requests>()
            .init_resource::<PendingLayerRequests>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingWorkspaceControls>()
            .init_resource::<WorkArea>()
            .init_resource::<EntityIndex>()
            .init_resource::<ActiveWindowGrab>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WorkspaceTilingState>()
            .init_resource::<SurfacePresentationSnapshot>()
            .init_resource::<workspace::RememberedOutputWorkspaceState>()
            .init_resource::<CommandHistoryState>()
            .init_resource::<commands::StartupActionState>()
            .init_resource::<PendingExternalCommandRequests>()
            .add_message::<WindowCreated>()
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
                        rebuild_entity_index_system,
                        commands::startup_action_queue_system,
                        commands::external_command_launch_system,
                        commands::command_history_system,
                        workspace::workspace_switch_system,
                        workspace::workspace_command_system,
                        workspace::output_workspace_housekeeping_system,
                        workspace::remember_output_workspace_routes_system,
                        workspace::sync_active_workspace_marker_system,
                        workspace::sync_workspace_disabled_state_system,
                        layer::arrange::layer_lifecycle_system,
                        layer::arrange::sync_layer_output_relationships_system,
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
                    )
                        .chain(),
                )
                    .chain(),
            );
    }
}
