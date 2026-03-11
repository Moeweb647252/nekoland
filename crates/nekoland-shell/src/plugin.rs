use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::LayoutSchedule;
use nekoland_ecs::events::{WindowClosed, WindowCreated, WindowMoved};
use nekoland_ecs::resources::{
    PendingLayerRequests, PendingPopupServerRequests, PendingWindowServerRequests,
    PendingWorkspaceServerRequests, PendingX11Requests, PendingXdgRequests, WorkArea,
};

use crate::{decorations, focus, layer, layout, workspace, x11, xdg};

#[derive(Debug, Default, Clone, Copy)]
pub struct ShellPlugin;

impl NekolandPlugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingXdgRequests>()
            .init_resource::<PendingX11Requests>()
            .init_resource::<PendingLayerRequests>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingWorkspaceServerRequests>()
            .init_resource::<WorkArea>()
            .add_message::<WindowCreated>()
            .add_message::<WindowClosed>()
            .add_message::<WindowMoved>()
            .add_systems(
                LayoutSchedule,
                (
                    workspace::workspace_switch_system,
                    workspace::workspace_command_system,
                    layer::arrange::layer_lifecycle_system,
                    xdg::toplevel::toplevel_lifecycle_system,
                    xdg::popup::popup_parent_close_system,
                    xdg::popup::popup_management_system,
                    xdg::configure::configure_sequence_system,
                    xdg::configure::window_geometry_request_system,
                    layer::arrange::layer_arrangement_system,
                    layer::arrange::work_area_system,
                    x11::xwayland::xwayland_bridge_system,
                    // Floating is the active layout strategy.
                    // tiling_layout_system and stacking_layout_system are not
                    // yet implemented and are excluded from the schedule.
                    // See layout/tiling.rs and layout/stacking.rs for the
                    // extension guide.
                    layout::floating::floating_layout_system,
                    layout::fullscreen::fullscreen_layout_system,
                    focus::window_focus_request_system,
                    focus::focus_management_system,
                    decorations::server_decoration_system,
                )
                    .chain(),
            );
    }
}
