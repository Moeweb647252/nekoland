use std::collections::BTreeMap;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{WindowLayout, XdgWindow};
use nekoland_ecs::resources::{
    PrimaryOutputState, UNASSIGNED_WORKSPACE_TILING_ID, WorkArea, WorkspaceTilingState,
};
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::{project_scene_geometry, resolve_output_state_for_workspace};

/// Tiling layout strategy backed by a workspace-scoped binary tile tree.
///
/// The current tree shape is rebuilt from stable leaf order whenever the tiled window set changes.
/// That keeps the runtime model tree-shaped and workspace-local now, while leaving room for future
/// split manipulation controls without reworking the shell/layout boundary again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TilingLayout;

/// Reconciles workspace-local tile trees and applies base geometry to all tiled windows.
pub fn tiling_layout_system(
    mut tiling: ResMut<WorkspaceTilingState>,
    mut windows: Query<WindowRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    work_area: Res<WorkArea>,
) {
    let tiled_surfaces = windows
        .iter()
        .filter(|window| {
            window.background.is_none() && matches!(*window.layout, WindowLayout::Tiled)
        })
        .map(|window| {
            (
                window.surface_id(),
                window_workspace_runtime_id(window.child_of, &workspaces)
                    .unwrap_or(UNASSIGNED_WORKSPACE_TILING_ID),
            )
        })
        .collect::<BTreeMap<_, _>>();

    tiling.retain_known(&tiled_surfaces);
    for (surface_id, workspace_id) in &tiled_surfaces {
        tiling.ensure_surface(*workspace_id, *surface_id);
    }

    let mut arranged = BTreeMap::new();
    for (workspace_id, tree) in &tiling.workspaces {
        let workspace_area = resolve_output_state_for_workspace(
            &outputs,
            Some(*workspace_id),
            primary_output.as_deref(),
        )
        .map(|(_, _, _, work_area)| WorkArea {
            x: work_area.x,
            y: work_area.y,
            width: work_area.width,
            height: work_area.height,
        })
        .unwrap_or(*work_area);
        arranged.extend(tree.arranged_geometry(&workspace_area));
    }
    for mut window in &mut windows {
        if window.background.is_some() {
            continue;
        }
        let Some(geometry) = arranged.get(&window.surface_id()) else {
            continue;
        };

        window.scene_geometry.x = geometry.x as isize;
        window.scene_geometry.y = geometry.y as isize;
        window.scene_geometry.width = geometry.width;
        window.scene_geometry.height = geometry.height;
        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces)
            .unwrap_or(UNASSIGNED_WORKSPACE_TILING_ID);
        if let Some((_, _, viewport, _)) = resolve_output_state_for_workspace(
            &outputs,
            Some(workspace_id),
            primary_output.as_deref(),
        ) {
            *window.geometry = project_scene_geometry(&window.scene_geometry, viewport);
        } else {
            *window.geometry = geometry.clone();
        }
    }

    tracing::trace!(workspaces = tiling.workspaces.len(), "tiling layout system tick");
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        BufferState, SurfaceGeometry, WindowLayout, WindowMode, WlSurfaceHandle, Workspace,
        WorkspaceId, XdgWindow,
    };
    use nekoland_ecs::resources::{WorkArea, WorkspaceTilingState};

    use super::tiling_layout_system;

    #[test]
    fn tiling_layout_splits_two_windows_into_columns() {
        let mut app = NekolandApp::new("tiling-layout-test");
        app.insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .insert_resource(WorkspaceTilingState::default())
            .inner_mut()
            .add_systems(LayoutSchedule, tiling_layout_system);

        let workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();

        let left = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 11 },
                    geometry: SurfaceGeometry { x: 32, y: 48, width: 300, height: 200 },
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                bevy_ecs::hierarchy::ChildOf(workspace),
            ))
            .id();
        let right = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 22 },
                    geometry: SurfaceGeometry { x: 400, y: 60, width: 300, height: 200 },
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                bevy_ecs::hierarchy::ChildOf(workspace),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let left_geometry = world.get::<SurfaceGeometry>(left).expect("left geometry");
        let right_geometry = world.get::<SurfaceGeometry>(right).expect("right geometry");

        assert_eq!(
            (left_geometry.x, left_geometry.y, left_geometry.width, left_geometry.height),
            (0, 0, 640, 720)
        );
        assert_eq!(
            (right_geometry.x, right_geometry.y, right_geometry.width, right_geometry.height),
            (640, 0, 640, 720)
        );
    }

    #[test]
    fn tiling_layout_keeps_workspace_trees_separate() {
        let mut app = NekolandApp::new("tiling-workspace-test");
        app.insert_resource(WorkArea { x: 0, y: 0, width: 900, height: 600 })
            .insert_resource(WorkspaceTilingState::default())
            .inner_mut()
            .add_systems(LayoutSchedule, tiling_layout_system);

        let workspace_one = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        let workspace_two = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false })
            .id();

        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 1, height: 1 },
                buffer: BufferState { attached: true, scale: 1 },
                window: XdgWindow::default(),
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                ..Default::default()
            },
            bevy_ecs::hierarchy::ChildOf(workspace_one),
        ));
        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 22 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 1, height: 1 },
                buffer: BufferState { attached: true, scale: 1 },
                window: XdgWindow::default(),
                layout: WindowLayout::Tiled,
                mode: WindowMode::Normal,
                ..Default::default()
            },
            bevy_ecs::hierarchy::ChildOf(workspace_two),
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let tiling = app
            .inner()
            .world()
            .get_resource::<WorkspaceTilingState>()
            .expect("tiling state should exist");
        assert_eq!(tiling.workspaces.len(), 2);
        assert_eq!(tiling.workspaces.get(&1).expect("workspace 1").leaf_surfaces, vec![11]);
        assert_eq!(tiling.workspaces.get(&2).expect("workspace 2").leaf_surfaces, vec![22]);
    }
}
