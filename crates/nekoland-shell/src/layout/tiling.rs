use std::collections::BTreeMap;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Local, Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::OutputId;
use nekoland_ecs::components::{WindowLayout, WindowMode, XdgWindow};
use nekoland_ecs::resources::{
    KeyboardFocusState, OutputControlHandle, PendingOutputControls, PendingTilingControl,
    PendingTilingControls, UNASSIGNED_WORKSPACE_TILING_ID, WaylandIngress, WindowStackingState,
    WorkArea, WorkspaceTilingState,
};
use nekoland_ecs::selectors::OutputName;
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WindowSnapshotRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::{
    preferred_primary_output_id, project_scene_geometry, resolve_output_state_for_workspace,
    scene_geometry_intersects_viewport,
};

/// Column/row tiled layout with snapped viewport semantics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TilingLayout;

/// Applies queued tiling mutations against the focused tiled surface and stages snapped viewport
/// moves through the normal output-control pipeline.
pub fn tiling_control_request_system(
    mut pending_tiling_controls: ResMut<PendingTilingControls>,
    mut tiling: ResMut<WorkspaceTilingState>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut stacking: ResMut<WindowStackingState>,
    mut pending_output_controls: ResMut<PendingOutputControls>,
    windows: Query<WindowSnapshotRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    wayland_ingress: Res<WaylandIngress>,
) {
    if pending_tiling_controls.is_empty() {
        return;
    }

    let primary_output_id = preferred_primary_output_id(Some(&wayland_ingress));
    let mut staged_viewports = BTreeMap::<String, (isize, isize)>::new();

    for control in pending_tiling_controls.take() {
        let Some((focused_surface_id, workspace_id)) =
            focused_tiled_surface_context(&keyboard_focus, &windows, &workspaces)
        else {
            continue;
        };

        match control {
            PendingTilingControl::FocusColumn { direction } => {
                if let Some(target_surface_id) =
                    tiling.focus_column(workspace_id, focused_surface_id, direction)
                {
                    keyboard_focus.focused_surface = Some(target_surface_id);
                    stacking.raise(workspace_id, target_surface_id);
                }
            }
            PendingTilingControl::FocusWindow { direction } => {
                if let Some(target_surface_id) =
                    tiling.focus_window(workspace_id, focused_surface_id, direction)
                {
                    keyboard_focus.focused_surface = Some(target_surface_id);
                    stacking.raise(workspace_id, target_surface_id);
                }
            }
            PendingTilingControl::MoveColumn { direction } => {
                if tiling.move_column(workspace_id, focused_surface_id, direction) {
                    stacking.raise(workspace_id, focused_surface_id);
                }
            }
            PendingTilingControl::MoveWindow { direction } => {
                if tiling.move_window(workspace_id, focused_surface_id, direction) {
                    stacking.raise(workspace_id, focused_surface_id);
                }
            }
            PendingTilingControl::ConsumeIntoColumn { direction } => {
                if tiling.consume_into_column(workspace_id, focused_surface_id, direction) {
                    stacking.raise(workspace_id, focused_surface_id);
                }
            }
            PendingTilingControl::ExpelFromColumn { direction } => {
                if tiling.expel_from_column(workspace_id, focused_surface_id, direction) {
                    stacking.raise(workspace_id, focused_surface_id);
                }
            }
            PendingTilingControl::PanViewport { direction } => {
                let Some((output_name, current_origin_x, current_origin_y, workspace_area)) =
                    tiling_output_context(
                        &outputs,
                        primary_output_id,
                        workspace_id,
                        &staged_viewports,
                    )
                else {
                    continue;
                };
                if let Some((target_x, target_y)) = tiling.snapped_viewport_after_pan(
                    workspace_id,
                    &workspace_area,
                    keyboard_focus.focused_surface,
                    current_origin_x,
                    current_origin_y,
                    direction,
                ) {
                    staged_viewports.insert(output_name, (target_x, target_y));
                }
            }
        }
    }

    stage_output_viewport_moves(&mut pending_output_controls, staged_viewports);
}

/// Reconciles workspace-local columns and applies base geometry to all tiled windows.
pub fn tiling_layout_system(
    mut tiling: ResMut<WorkspaceTilingState>,
    mut windows: Query<WindowRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    wayland_ingress: Res<WaylandIngress>,
    work_area: Res<WorkArea>,
) {
    let primary_output_id = preferred_primary_output_id(Some(&wayland_ingress));
    let mut tiled_surfaces = BTreeMap::<u64, u32>::new();
    let mut discovered_by_workspace = BTreeMap::<u32, Vec<u64>>::new();

    for window in windows.iter() {
        if !window.role.is_managed() || !matches!(*window.layout, WindowLayout::Tiled) {
            continue;
        }
        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces)
            .unwrap_or(UNASSIGNED_WORKSPACE_TILING_ID);
        tiled_surfaces.insert(window.surface_id(), workspace_id);
        discovered_by_workspace.entry(workspace_id).or_default().push(window.surface_id());
    }

    tiling.retain_known(&tiled_surfaces);
    for (workspace_id, surface_ids) in discovered_by_workspace {
        for surface_id in surface_ids {
            tiling.ensure_surface(workspace_id, surface_id);
        }
    }

    let mut arranged = BTreeMap::new();
    for workspace_id in tiled_surfaces.values().copied().collect::<std::collections::BTreeSet<_>>() {
        let workspace_area =
            resolve_output_state_for_workspace(&outputs, Some(workspace_id), primary_output_id)
                .map(|(_, _, _, work_area)| WorkArea {
                    x: work_area.x,
                    y: work_area.y,
                    width: work_area.width,
                    height: work_area.height,
                })
                .unwrap_or(*work_area);
        if let Some(layout) = tiling.workspaces.get(&workspace_id) {
            arranged.extend(layout.arranged_geometry(&workspace_area));
        }
    }

    for mut window in &mut windows {
        if !window.role.is_managed() {
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
        if let Some((_, _, viewport, _)) =
            resolve_output_state_for_workspace(&outputs, Some(workspace_id), primary_output_id)
        {
            *window.geometry = project_scene_geometry(&window.scene_geometry, viewport);
        } else {
            *window.geometry = geometry.clone();
        }
    }

    tracing::trace!(workspaces = tiling.workspaces.len(), "tiling layout system tick");
}

/// When focus lands on a tiled surface that is entirely outside the viewport, queue one snapped
/// viewport move to reveal the focused tile on the next frame.
pub fn tiling_focus_auto_align_system(
    mut previous_focus: Local<Option<u64>>,
    keyboard_focus: Res<KeyboardFocusState>,
    tiling: Res<WorkspaceTilingState>,
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    windows: Query<WindowSnapshotRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
    wayland_ingress: Res<WaylandIngress>,
    mut pending_output_controls: ResMut<PendingOutputControls>,
) {
    let focused_surface = keyboard_focus.focused_surface;
    let focus_changed = focused_surface != *previous_focus;
    *previous_focus = focused_surface;
    if !focus_changed {
        return;
    }

    let Some(focused_surface) = focused_surface else {
        return;
    };
    let Some(window) = windows.iter().find(|window| window.surface_id() == focused_surface) else {
        return;
    };
    if !window.role.is_managed()
        || *window.layout != WindowLayout::Tiled
        || *window.mode == WindowMode::Hidden
        || window.management_hints.helper_surface
    {
        return;
    }

    let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces)
        .unwrap_or(UNASSIGNED_WORKSPACE_TILING_ID);
    let primary_output_id = preferred_primary_output_id(Some(&wayland_ingress));
    let Some((_, output, viewport, output_work_area)) =
        resolve_output_state_for_workspace(&outputs, Some(workspace_id), primary_output_id)
    else {
        return;
    };
    let Some(output_name) = resolve_output_name_for_workspace(&outputs, workspace_id, primary_output_id)
    else {
        return;
    };
    if scene_geometry_intersects_viewport(window.scene_geometry, viewport, output) {
        return;
    }

    let workspace_area = WorkArea {
        x: output_work_area.x,
        y: output_work_area.y,
        width: output_work_area.width,
        height: output_work_area.height,
    };
    let Some((target_x, target_y)) =
        tiling.snapped_viewport_for_surface(workspace_id, &workspace_area, focused_surface)
    else {
        return;
    };
    pending_output_controls.named(OutputName::from(output_name)).move_viewport_to(target_x, target_y);
}

fn focused_tiled_surface_context(
    keyboard_focus: &KeyboardFocusState,
    windows: &Query<WindowSnapshotRuntime, (With<XdgWindow>, Allow<Disabled>)>,
    workspaces: &Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
) -> Option<(u64, u32)> {
    let focused_surface_id = keyboard_focus.focused_surface?;
    let window = windows.iter().find(|window| window.surface_id() == focused_surface_id)?;
    if !window.role.is_managed()
        || *window.layout != WindowLayout::Tiled
        || *window.mode == WindowMode::Hidden
        || window.management_hints.helper_surface
    {
        return None;
    }
    let workspace_id = window_workspace_runtime_id(window.child_of, workspaces)
        .unwrap_or(UNASSIGNED_WORKSPACE_TILING_ID);
    Some((focused_surface_id, workspace_id))
}

fn tiling_output_context(
    outputs: &Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    primary_output_id: Option<OutputId>,
    workspace_id: u32,
    staged_viewports: &BTreeMap<String, (isize, isize)>,
) -> Option<(String, isize, isize, WorkArea)> {
    let (_, _, viewport, work_area) =
        resolve_output_state_for_workspace(outputs, Some(workspace_id), primary_output_id)?;
    let output_name = resolve_output_name_for_workspace(outputs, workspace_id, primary_output_id)?;
    let (origin_x, origin_y) =
        staged_viewports.get(&output_name).copied().unwrap_or((viewport.origin_x, viewport.origin_y));
    Some((
        output_name,
        origin_x,
        origin_y,
        WorkArea {
            x: work_area.x,
            y: work_area.y,
            width: work_area.width,
            height: work_area.height,
        },
    ))
}

fn stage_output_viewport_moves(
    pending_output_controls: &mut PendingOutputControls,
    staged_viewports: BTreeMap<String, (isize, isize)>,
) {
    for (output_name, (x, y)) in staged_viewports {
        let mut control: OutputControlHandle<'_> =
            pending_output_controls.named(OutputName::from(output_name));
        control.move_viewport_to(x, y);
    }
}

fn resolve_output_name_for_workspace(
    outputs: &Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    workspace_id: u32,
    primary_output_id: Option<OutputId>,
) -> Option<String> {
    if let Some((_, output)) = outputs.iter().find(|(_, output)| {
        output
            .current_workspace
            .as_ref()
            .is_some_and(|current_workspace| current_workspace.workspace.0 == workspace_id)
    }) {
        return Some(output.name().to_owned());
    }

    if let Some(primary_output_id) = primary_output_id
        && let Some((_, output)) = outputs.iter().find(|(_, output)| output.id() == primary_output_id)
    {
        return Some(output.name().to_owned());
    }

    outputs.iter().next().map(|(_, output)| output.name().to_owned())
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        BufferState, OutputCurrentWorkspace, OutputDevice, OutputKind, OutputProperties,
        OutputViewport, SurfaceGeometry, WindowLayout, WindowMode, WlSurfaceHandle, Workspace,
        WorkspaceId, XdgWindow,
    };
    use nekoland_ecs::resources::{
        KeyboardFocusState, PendingOutputControls, PendingTilingControls,
        TilingPanDirection, WaylandIngress, WindowStackingState, WorkArea, WorkspaceTilingState,
    };

    use super::{tiling_control_request_system, tiling_layout_system};

    #[test]
    fn tiling_layout_places_new_tiled_windows_in_full_width_columns() {
        let mut app = NekolandApp::new("tiling-layout-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
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
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 300, height: 200 },
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                ChildOf(workspace),
            ))
            .id();
        let right = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 22 },
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 300, height: 200 },
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                ChildOf(workspace),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let left_geometry = world.get::<SurfaceGeometry>(left).expect("left geometry");
        let right_geometry = world.get::<SurfaceGeometry>(right).expect("right geometry");
        assert_eq!(
            (left_geometry.x, left_geometry.y, left_geometry.width, left_geometry.height),
            (0, 0, 1280, 720)
        );
        assert_eq!(
            (right_geometry.x, right_geometry.y, right_geometry.width, right_geometry.height),
            (1280, 0, 1280, 720)
        );
    }

    #[test]
    fn tiling_control_pan_stages_snapped_viewport_move() {
        let mut app = NekolandApp::new("tiling-pan-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .insert_resource(WorkspaceTilingState::default())
            .insert_resource(WindowStackingState::default())
            .insert_resource(KeyboardFocusState { focused_surface: Some(22) })
            .insert_resource(PendingTilingControls::default())
            .insert_resource(PendingOutputControls::default())
            .inner_mut()
            .add_systems(LayoutSchedule, (tiling_layout_system, tiling_control_request_system).chain());

        let workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        app.inner_mut().world_mut().spawn((
            OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Virtual".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                viewport: OutputViewport { origin_x: 0, origin_y: 0 },
                ..Default::default()
            },
            OutputCurrentWorkspace { workspace: WorkspaceId(1) },
        ));
        for surface_id in [11, 22] {
            app.inner_mut().world_mut().spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: surface_id },
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 300, height: 200 },
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                ChildOf(workspace),
            ));
        }
        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingTilingControls>()
            .api()
            .pan_viewport(TilingPanDirection::Right);

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let controls = app.inner().world().resource::<PendingOutputControls>();
        assert_eq!(controls.as_slice().len(), 1);
        assert_eq!(controls.as_slice()[0].viewport_origin.as_ref().map(|origin| origin.x), Some(1280));
    }

}
