use std::collections::BTreeMap;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Query, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::XdgWindow;
use nekoland_ecs::resources::UNASSIGNED_WORKSPACE_STACK_ID;
use nekoland_ecs::resources::WindowStackingState;
use nekoland_ecs::views::{WindowFocusRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

/// Z-order bookkeeping that keeps shell-visible windows in a stable back-to-front stack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackingLayout;

/// Reconciles the managed window stack after lifecycle/control systems have updated the window set.
pub fn stacking_layout_system(
    mut stacking: ResMut<WindowStackingState>,
    windows: Query<WindowFocusRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
) {
    let known_surfaces = windows
        .iter()
        .map(|window| {
            (
                window.surface_id(),
                window_workspace_runtime_id(window.child_of, &workspaces)
                    .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
            )
        })
        .collect::<BTreeMap<_, _>>();
    stacking.retain_known(&known_surfaces);

    for window in &windows {
        stacking.ensure(
            window_workspace_runtime_id(window.child_of, &workspaces)
                .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
            window.surface_id(),
        );
    }

    tracing::trace!(workspaces = stacking.workspaces.len(), "stacking layout system tick");
}
