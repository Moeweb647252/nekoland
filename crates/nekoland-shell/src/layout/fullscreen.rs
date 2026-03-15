use bevy_ecs::prelude::{Query, Res, With};
use nekoland_ecs::components::{WindowMode, XdgWindow};
use nekoland_ecs::resources::{PrimaryOutputState, WorkArea};
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::resolve_output_state_for_workspace;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullscreenLayout;

/// Applies fullscreen and maximized geometry after layout/work-area state has been updated.
pub fn fullscreen_layout_system(
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    mut windows: Query<WindowRuntime, With<XdgWindow>>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    work_area: Res<WorkArea>,
) {
    for mut window in &mut windows {
        if window.background.is_some() {
            continue;
        }
        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces);
        let output_state =
            resolve_output_state_for_workspace(&outputs, workspace_id, primary_output.as_deref());
        match *window.mode {
            WindowMode::Fullscreen => {
                let Some((_, output, _, _)) = output_state.as_ref() else {
                    continue;
                };
                window.geometry.x = 0;
                window.geometry.y = 0;
                window.geometry.width = output.width.max(1);
                window.geometry.height = output.height.max(1);
            }
            WindowMode::Maximized => {
                let window_work_area =
                    output_state.as_ref().map_or(*work_area, |(_, _, _, work_area)| WorkArea {
                        x: work_area.x,
                        y: work_area.y,
                        width: work_area.width,
                        height: work_area.height,
                    });
                // Keep a small inset so maximized windows still leave room for compositor-side
                // borders and do not visually merge into the output edge.
                window.geometry.x = window_work_area.x + 16;
                window.geometry.y = window_work_area.y + 16;
                window.geometry.width = window_work_area.width.saturating_sub(32).max(1);
                window.geometry.height = window_work_area.height.saturating_sub(32).max(1);
            }
            _ => {}
        }
    }

    tracing::trace!("fullscreen layout system tick");
}
