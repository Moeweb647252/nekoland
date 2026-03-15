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
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
) {
    let known_surfaces = windows
        .iter()
        .filter(|window| window.role.is_managed())
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
        if !window.role.is_managed() {
            continue;
        }
        stacking.ensure(
            window_workspace_runtime_id(window.child_of, &workspaces)
                .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
            window.surface_id(),
        );
    }

    tracing::trace!(workspaces = stacking.workspaces.len(), "stacking layout system tick");
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::entity_disabling::Disabled;
    use bevy_ecs::hierarchy::ChildOf;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{WlSurfaceHandle, Workspace, WorkspaceId};
    use nekoland_ecs::resources::{UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState};

    use super::stacking_layout_system;

    #[test]
    fn stacking_preserves_workspace_bucket_for_disabled_workspace_windows() {
        let mut app = NekolandApp::new("stacking-disabled-workspace-test");
        app.insert_resource(WindowStackingState {
            workspaces: BTreeMap::from([(2, vec![11, 22])]),
        })
        .inner_mut()
        .add_systems(LayoutSchedule, stacking_layout_system);

        let workspace = app
            .inner_mut()
            .world_mut()
            .spawn((
                Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false },
                Disabled,
            ))
            .id();
        app.inner_mut().world_mut().spawn((
            WindowBundle { surface: WlSurfaceHandle { id: 11 }, ..Default::default() },
            ChildOf(workspace),
        ));
        app.inner_mut().world_mut().spawn((
            WindowBundle { surface: WlSurfaceHandle { id: 22 }, ..Default::default() },
            ChildOf(workspace),
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let stacking = app.inner().world().resource::<WindowStackingState>();
        assert_eq!(stacking.workspaces.get(&2), Some(&vec![11, 22]));
        assert!(
            !stacking.workspaces.contains_key(&UNASSIGNED_WORKSPACE_STACK_ID),
            "disabled workspace windows should not be remapped into the unassigned stack: {stacking:?}"
        );
    }
}
