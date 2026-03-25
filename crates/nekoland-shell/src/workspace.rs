//! Workspace creation, switching, destruction, and output/workspace routing.
//!
//! The shell keeps workspaces authoritative and derives output routing from shell policy plus the
//! latest platform-facing output snapshot state.

use bevy_ecs::change_detection::DetectChanges;
use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::lifecycle::RemovedComponents;
use bevy_ecs::prelude::{Commands, Entity, Has, Query, Res, ResMut, Resource, With};
use bevy_ecs::query::{Added, Allow, Changed, Or};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{
    ActiveWorkspace, OutputCurrentWorkspace, OutputDevice, OutputId, Workspace, WorkspaceId,
    XdgWindow,
};
use nekoland_ecs::resources::{
    FocusedOutputState, PendingWorkspaceControls, WaylandIngress, WorkspaceControl,
};
use nekoland_ecs::selectors::{WorkspaceLookup, WorkspaceName, WorkspaceSelector};
use nekoland_ecs::views::{OutputRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_in_workspace;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Marker type documenting the shell workspace subsystem.
pub struct WorkspaceManager;

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
/// Remembers which workspace last belonged to each output across output lifecycle changes.
pub struct RememberedOutputWorkspaceState {
    /// Mapping from runtime output id to its last assigned workspace id.
    pub by_id: std::collections::BTreeMap<OutputId, WorkspaceId>,
    /// Mapping from output name to runtime output id for reconnect-friendly lookup.
    pub ids_by_name: std::collections::BTreeMap<String, OutputId>,
}

impl RememberedOutputWorkspaceState {
    /// Returns the remembered workspace for the provided output name, when one exists.
    pub fn workspace_for_output_name(&self, output_name: &str) -> Option<WorkspaceId> {
        self.ids_by_name.get(output_name).and_then(|output_id| self.by_id.get(output_id)).copied()
    }

    /// Records the most recent workspace assignment for one output.
    pub fn remember(
        &mut self,
        output_id: OutputId,
        output_name: String,
        workspace_id: WorkspaceId,
    ) {
        self.ids_by_name.insert(output_name, output_id);
        self.by_id.insert(output_id, workspace_id);
    }
}

type WorkspaceWindows<'w, 's> =
    Query<'w, 's, (Entity, Option<&'static ChildOf>), (With<XdgWindow>, Allow<Disabled>)>;
type WorkspaceOutputs<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static OutputId, &'static OutputDevice, Option<&'static OutputCurrentWorkspace>),
>;
type WorkspaceEntities<'w, 's> = Query<'w, 's, (), With<Workspace>>;
type ActiveWorkspaceEntities<'w, 's> = Query<'w, 's, (), With<ActiveWorkspace>>;
type WorkspaceAdditions<'w, 's> = Query<'w, 's, (), Added<Workspace>>;
type WorkspaceOutputRouteChanges<'w, 's> = Query<
    'w,
    's,
    (),
    Or<(
        Added<OutputDevice>,
        Changed<OutputDevice>,
        Added<OutputCurrentWorkspace>,
        Changed<OutputCurrentWorkspace>,
    )>,
>;

#[derive(SystemParam)]
/// System parameters required to apply staged workspace controls.
pub struct WorkspaceCommandParams<'w, 's> {
    pending_workspace_controls: ResMut<'w, PendingWorkspaceControls>,
    wayland_ingress: Res<'w, WaylandIngress>,
    focused_output: Res<'w, FocusedOutputState>,
    workspaces: Query<'w, 's, (Entity, WorkspaceRuntime), Allow<Disabled>>,
    outputs: WorkspaceOutputs<'w, 's>,
    windows: WorkspaceWindows<'w, 's>,
}

#[derive(SystemParam)]
pub(crate) struct WorkspaceReconciliationState<'w, 's> {
    pending_workspace_controls: Res<'w, PendingWorkspaceControls>,
    workspaces: WorkspaceEntities<'w, 's>,
    active_workspaces: ActiveWorkspaceEntities<'w, 's>,
    workspace_additions: WorkspaceAdditions<'w, 's>,
    output_route_changes: WorkspaceOutputRouteChanges<'w, 's>,
    wayland_ingress: Res<'w, WaylandIngress>,
    focused_output: Res<'w, FocusedOutputState>,
    removed_workspaces: RemovedComponents<'w, 's, Workspace>,
    removed_outputs: RemovedComponents<'w, 's, OutputDevice>,
    removed_output_routes: RemovedComponents<'w, 's, OutputCurrentWorkspace>,
}

/// Ensures the compositor always has at least one active workspace entity.
pub fn workspace_switch_system(
    mut commands: Commands,
    mut workspaces: Query<(WorkspaceRuntime, Has<Disabled>)>,
) {
    if workspaces.is_empty() {
        commands.spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true });
        tracing::trace!("seeded initial workspace");
        return;
    }

    if workspaces.iter().all(|(workspace, _)| !workspace.is_active()) {
        let fallback_id =
            workspaces.iter().map(|(workspace, _)| workspace.id().0).min().unwrap_or(1);
        for (mut workspace, _) in &mut workspaces {
            workspace.workspace.active = workspace.id().0 == fallback_id;
        }
    }

    tracing::trace!(count = workspaces.iter().count(), "workspace housekeeping tick");
}

pub(crate) fn workspace_reconciliation_needed(state: WorkspaceReconciliationState<'_, '_>) -> bool {
    let WorkspaceReconciliationState {
        pending_workspace_controls,
        workspaces,
        active_workspaces,
        workspace_additions,
        output_route_changes,
        wayland_ingress,
        focused_output,
        removed_workspaces,
        removed_outputs,
        removed_output_routes,
    } = state;

    if !pending_workspace_controls.is_empty() || workspaces.is_empty() {
        return true;
    }

    if active_workspaces.iter().take(2).count() != 1 {
        return true;
    }

    !workspace_additions.is_empty()
        || !output_route_changes.is_empty()
        || !removed_workspaces.is_empty()
        || !removed_outputs.is_empty()
        || !removed_output_routes.is_empty()
        || wayland_ingress.is_changed()
        || focused_output.is_changed()
}

/// Applies create/switch/destroy requests and keeps child window relationships in sync when a
/// workspace disappears.
pub fn workspace_command_system(
    mut commands: Commands,
    mut workspace_commands: WorkspaceCommandParams<'_, '_>,
) {
    let mut snapshot = workspace_commands
        .workspaces
        .iter_mut()
        .map(|(entity, workspace)| {
            (entity, workspace.id().0, workspace.name().to_owned(), workspace.is_active())
        })
        .collect::<Vec<_>>();
    let output_snapshot = workspace_commands
        .outputs
        .iter()
        .map(|(entity, output_id, output, current_workspace)| {
            (
                entity,
                *output_id,
                output.name.clone(),
                current_workspace.map(|current_workspace| current_workspace.workspace.0),
            )
        })
        .collect::<Vec<_>>();
    let primary_output_id =
        crate::viewport::preferred_primary_output_id(Some(&workspace_commands.wayland_ingress));
    let focused_output_id = workspace_commands.focused_output.id;

    for control in workspace_commands.pending_workspace_controls.take() {
        match control {
            WorkspaceControl::Create { target } => {
                if resolve_workspace_lookup(&snapshot, &target).is_some() {
                    continue;
                }

                let (workspace_id, workspace_name) = create_workspace_identity(&snapshot, &target);
                let workspace_entity = commands
                    .spawn(Workspace {
                        id: WorkspaceId(workspace_id),
                        name: workspace_name.clone(),
                        active: false,
                    })
                    .id();
                snapshot.push((workspace_entity, workspace_id, workspace_name, false));
            }
            WorkspaceControl::SwitchOrCreate { target } => {
                let existing_target = resolve_workspace_lookup(&snapshot, &target)
                    .map(|(entity, id, name, _)| (entity, id, name));
                let (target_entity, target_id, target_name) =
                    existing_target.unwrap_or_else(|| {
                        let (workspace_id, workspace_name) =
                            create_workspace_identity(&snapshot, &target);
                        let workspace_entity = commands
                            .spawn(Workspace {
                                id: WorkspaceId(workspace_id),
                                name: workspace_name.clone(),
                                active: true,
                            })
                            .id();
                        (workspace_entity, workspace_id, workspace_name)
                    });

                if let Some(target_output_entity) = resolve_workspace_control_output_entity(
                    &output_snapshot,
                    focused_output_id,
                    primary_output_id,
                ) {
                    commands
                        .entity(target_output_entity)
                        .insert(OutputCurrentWorkspace { workspace: WorkspaceId(target_id) });
                } else {
                    for (entity, mut active_workspace) in &mut workspace_commands.workspaces {
                        active_workspace.workspace.active = entity == target_entity;
                    }
                }

                snapshot.retain(|(entity, _, _, _)| *entity != target_entity);
                snapshot.push((target_entity, target_id, target_name, true));
                for entry in &mut snapshot {
                    entry.3 = entry.1 == target_id;
                }
            }
            WorkspaceControl::Destroy { target } => {
                if snapshot.len() <= 1 {
                    continue;
                }

                let Some((target_entity, target_id, _, target_active)) =
                    resolve_workspace_selector(&snapshot, &target)
                else {
                    continue;
                };

                let (fallback_entity, fallback_id) = snapshot
                    .iter()
                    .filter(|(_, id, _, _)| *id != target_id)
                    .find(|(_, _, _, active)| *active || target_active)
                    .map(|(entity, id, _, _)| (Some(*entity), *id))
                    .unwrap_or_else(|| {
                        snapshot
                            .iter()
                            .find(|(_, id, _, _)| *id != target_id)
                            .map(|(entity, id, _, _)| (Some(*entity), *id))
                            .unwrap_or((None, 1))
                    });

                for (window_entity, child_of) in &workspace_commands.windows {
                    if !window_in_workspace(child_of, target_entity) {
                        continue;
                    }

                    if let Some(fallback_entity) = fallback_entity {
                        commands.entity(window_entity).insert(ChildOf(fallback_entity));
                    }
                }

                for (output_entity, _, _, current_workspace) in &workspace_commands.outputs {
                    if current_workspace
                        .is_some_and(|current_workspace| current_workspace.workspace.0 == target_id)
                        && let Some(fallback_entity) = fallback_entity
                        && let Some((_, fallback_workspace_id, _, _)) =
                            snapshot.iter().find(|(entity, _, _, _)| *entity == fallback_entity)
                    {
                        commands.entity(output_entity).insert(OutputCurrentWorkspace {
                            workspace: WorkspaceId(*fallback_workspace_id),
                        });
                    }
                }

                for (_, mut existing_workspace) in &mut workspace_commands.workspaces {
                    if existing_workspace.id().0 == target_id {
                        continue;
                    }
                    existing_workspace.workspace.active = existing_workspace.id().0 == fallback_id;
                }

                commands.entity(target_entity).despawn();

                snapshot.retain(|(_, id, _, _)| *id != target_id);
                for entry in &mut snapshot {
                    entry.3 = entry.1 == fallback_id;
                }
            }
        }
    }
}

/// Ensures every output points at one visible workspace and that one workspace is not projected on
/// multiple outputs at once.
pub fn output_workspace_housekeeping_system(
    mut commands: Commands,
    mut remembered_outputs: ResMut<RememberedOutputWorkspaceState>,
    outputs: Query<(Entity, &OutputId, &OutputDevice, Option<&OutputCurrentWorkspace>)>,
    mut workspaces: Query<(Entity, WorkspaceRuntime), Allow<Disabled>>,
) {
    if outputs.is_empty() {
        return;
    }

    let mut snapshot = workspaces
        .iter_mut()
        .map(|(entity, workspace)| (entity, workspace.id().0, workspace.name().to_owned()))
        .collect::<Vec<_>>();
    let mut assigned = std::collections::BTreeSet::new();
    let mut needs_assignment = Vec::new();

    for (output_entity, output_id, output, current_workspace) in &outputs {
        let Some(workspace_id) =
            current_workspace.map(|current_workspace| current_workspace.workspace.0)
        else {
            needs_assignment.push((output_entity, *output_id, output.name.clone()));
            continue;
        };

        if snapshot.iter().any(|(_, id, _)| *id == workspace_id) && assigned.insert(workspace_id) {
            continue;
        }

        needs_assignment.push((output_entity, *output_id, output.name.clone()));
    }

    for (output_entity, output_id, output_name) in needs_assignment {
        let remembered_workspace = remembered_outputs
            .workspace_for_output_name(&output_name)
            .map(|workspace_id| workspace_id.0)
            .filter(|workspace_id| !assigned.contains(workspace_id))
            .and_then(|workspace_id| {
                snapshot
                    .iter()
                    .find(|(_, id, _)| *id == workspace_id)
                    .map(|(entity, id, name)| (*entity, *id, name.clone()))
            });
        let next_workspace = remembered_workspace.unwrap_or_else(|| {
            snapshot
                .iter()
                .find(|(_, id, _)| !assigned.contains(id))
                .map(|(entity, id, name)| (*entity, *id, name.clone()))
                .unwrap_or_else(|| {
                    let workspace_id = next_workspace_id_with_names(&snapshot);
                    let workspace_name = workspace_id.to_string();
                    let workspace_entity = commands
                        .spawn(Workspace {
                            id: WorkspaceId(workspace_id),
                            name: workspace_name.clone(),
                            active: false,
                        })
                        .id();
                    snapshot.push((workspace_entity, workspace_id, workspace_name.clone()));
                    (workspace_entity, workspace_id, workspace_name)
                })
        });

        assigned.insert(next_workspace.1);
        commands
            .entity(output_entity)
            .insert(OutputCurrentWorkspace { workspace: WorkspaceId(next_workspace.1) });
        remembered_outputs.remember(output_id, output_name, WorkspaceId(next_workspace.1));
    }
}

/// Persists the current output-to-workspace mapping for later reconnect or re-enable flows.
pub fn remember_output_workspace_routes_system(
    outputs: Query<(&OutputId, &OutputDevice, Option<&OutputCurrentWorkspace>)>,
    mut remembered_outputs: ResMut<RememberedOutputWorkspaceState>,
) {
    for (output_id, output, current_workspace) in &outputs {
        if let Some(current_workspace) = current_workspace {
            remembered_outputs.remember(
                *output_id,
                output.name.clone(),
                current_workspace.workspace,
            );
        }
    }
}

/// Mirrors the active workspace selection into Bevy's `Disabled` hierarchy state so downstream
/// systems can ignore inactive workspaces without filtering every query manually.
pub fn sync_active_workspace_marker_system(
    mut commands: Commands,
    wayland_ingress: Res<WaylandIngress>,
    focused_output: Res<FocusedOutputState>,
    outputs: Query<OutputRuntime>,
    mut workspaces: Query<(Entity, WorkspaceRuntime, Has<ActiveWorkspace>), Allow<Disabled>>,
) {
    let primary_output_id = crate::viewport::preferred_primary_output_id(Some(&wayland_ingress));
    let focused_output_id = focused_output.id;
    let active_workspace_id = resolve_preferred_workspace_id(
        &outputs,
        focused_output_id,
        primary_output_id,
    );
    let active_workspace = active_workspace_id
        .and_then(|active_workspace_id| {
            workspaces
                .iter()
                .find(|(_, workspace, _)| workspace.id().0 == active_workspace_id)
                .map(|(entity, _, _)| entity)
        })
        .or_else(|| {
            workspaces
                .iter()
                .min_by_key(|(_, workspace, _)| workspace.id().0)
                .map(|(entity, _, _)| entity)
        });

    for (entity, mut workspace, has_active_marker) in &mut workspaces {
        let is_active = Some(entity) == active_workspace;
        if workspace.workspace.active != is_active {
            workspace.workspace.active = is_active;
        }

        if is_active {
            if !has_active_marker {
                commands.entity(entity).insert(ActiveWorkspace);
            }
        } else if has_active_marker {
            commands.entity(entity).remove::<ActiveWorkspace>();
        }
    }
}

/// Mirrors the active workspace selection into Bevy's `Disabled` hierarchy state so downstream
/// systems can ignore inactive workspaces without filtering every query manually.
pub fn sync_workspace_disabled_state_system(
    mut commands: Commands,
    outputs: Query<OutputRuntime>,
    workspaces: Query<(Entity, WorkspaceRuntime, Has<Disabled>)>,
) {
    let mut visible_workspaces = outputs
        .iter()
        .filter_map(|output| {
            output.current_workspace.as_ref().map(|current_workspace| current_workspace.workspace.0)
        })
        .collect::<std::collections::BTreeSet<_>>();
    if visible_workspaces.is_empty()
        && let Some((_, workspace, _)) =
            workspaces.iter().min_by_key(|(_, workspace, _)| workspace.id().0)
    {
        visible_workspaces.insert(workspace.id().0);
    }

    for (entity, _, is_disabled) in &workspaces {
        let visible = workspaces
            .get(entity)
            .ok()
            .is_some_and(|(_, workspace, _)| visible_workspaces.contains(&workspace.id().0));
        if visible {
            if is_disabled {
                commands.entity(entity).remove_recursive::<Children, Disabled>();
            }
        } else if !is_disabled {
            commands.entity(entity).insert_recursive::<Children>(Disabled);
        }
    }
}

/// Generates a fresh numeric workspace id when the user addressed a workspace by a non-numeric
/// name or when the requested numeric id is already taken.
fn next_workspace_id(snapshot: &[(Entity, u32, String, bool)]) -> u32 {
    snapshot.iter().map(|(_, id, _, _)| *id).max().unwrap_or(0).saturating_add(1)
}

fn next_workspace_id_with_names(snapshot: &[(Entity, u32, String)]) -> u32 {
    snapshot.iter().map(|(_, id, _)| *id).max().unwrap_or(0).saturating_add(1)
}

fn resolve_workspace_lookup(
    snapshot: &[(Entity, u32, String, bool)],
    lookup: &WorkspaceLookup,
) -> Option<(Entity, u32, String, bool)> {
    snapshot
        .iter()
        .find(|(_, id, name, _)| match lookup {
            WorkspaceLookup::Id(workspace_id) => *id == workspace_id.0,
            WorkspaceLookup::Name(workspace_name) => name == workspace_name.as_str(),
        })
        .cloned()
}

fn resolve_workspace_selector(
    snapshot: &[(Entity, u32, String, bool)],
    selector: &WorkspaceSelector,
) -> Option<(Entity, u32, String, bool)> {
    match selector {
        WorkspaceSelector::Active => snapshot
            .iter()
            .find(|(_, _, _, active)| *active)
            .cloned()
            .or_else(|| snapshot.iter().min_by_key(|(_, id, _, _)| *id).cloned()),
        WorkspaceSelector::Id(workspace_id) => {
            resolve_workspace_lookup(snapshot, &WorkspaceLookup::Id(*workspace_id))
        }
        WorkspaceSelector::Name(workspace_name) => {
            resolve_workspace_lookup(snapshot, &WorkspaceLookup::Name(workspace_name.clone()))
        }
    }
}

fn create_workspace_identity(
    snapshot: &[(Entity, u32, String, bool)],
    lookup: &WorkspaceLookup,
) -> (u32, String) {
    match lookup {
        WorkspaceLookup::Id(workspace_id) => (workspace_id.0, workspace_id.0.to_string()),
        WorkspaceLookup::Name(WorkspaceName(workspace_name)) => {
            (next_workspace_id(snapshot), workspace_name.clone())
        }
    }
}

fn resolve_workspace_control_output_entity(
    outputs: &[(Entity, OutputId, String, Option<u32>)],
    focused_output_id: Option<OutputId>,
    primary_output: Option<OutputId>,
) -> Option<Entity> {
    if let Some(output_id) = focused_output_id
        && let Some((entity, _, _, _)) =
            outputs.iter().find(|(_, candidate_id, _, _)| *candidate_id == output_id)
    {
        return Some(*entity);
    }

    if let Some(output_id) = primary_output
        && let Some((entity, _, _, _)) =
            outputs.iter().find(|(_, candidate_id, _, _)| *candidate_id == output_id)
    {
        return Some(*entity);
    }

    outputs.first().map(|(entity, _, _, _)| *entity)
}

fn resolve_preferred_workspace_id(
    outputs: &Query<OutputRuntime>,
    focused_output_id: Option<OutputId>,
    primary_output: Option<OutputId>,
) -> Option<u32> {
    if let Some(output_id) = focused_output_id
        && let Some(workspace_id) =
            outputs.iter().find(|output| output.id() == output_id).and_then(|output| {
                output
                    .current_workspace
                    .as_ref()
                    .map(|current_workspace| current_workspace.workspace.0)
            })
    {
        return Some(workspace_id);
    }

    if let Some(output_id) = primary_output
        && let Some(workspace_id) =
            outputs.iter().find(|output| output.id() == output_id).and_then(|output| {
                output
                    .current_workspace
                    .as_ref()
                    .map(|current_workspace| current_workspace.workspace.0)
            })
    {
        return Some(workspace_id);
    }

    outputs.iter().find_map(|output| {
        output.current_workspace.as_ref().map(|current_workspace| current_workspace.workspace.0)
    })
}

#[cfg(test)]
mod tests {
    use bevy_ecs::entity_disabling::Disabled;
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::{ResMut, Resource};
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        ActiveWorkspace, BorderTheme, BufferState, OutputCurrentWorkspace, OutputDevice, OutputId,
        OutputKind, OutputProperties, ServerDecoration, SurfaceGeometry, WindowAnimation,
        WindowLayout, WindowMode, WlSurfaceHandle, Workspace, WorkspaceId, XdgWindow,
    };
    use nekoland_ecs::resources::{
        EntityIndex, FocusedOutputState, PendingWorkspaceControls, PrimaryOutputState,
        WaylandIngress,
    };

    use super::{
        RememberedOutputWorkspaceState, output_workspace_housekeeping_system,
        remember_output_workspace_routes_system, sync_active_workspace_marker_system,
        sync_workspace_disabled_state_system, workspace_command_system,
        workspace_reconciliation_needed,
    };

    #[derive(Debug, Default, Resource)]
    struct WorkspaceReconciliationAudit(u32);

    fn record_workspace_reconciliation(mut audit: ResMut<WorkspaceReconciliationAudit>) {
        audit.0 += 1;
    }

    #[test]
    fn destroying_workspace_reparents_windows_to_fallback_workspace() {
        let mut app = NekolandApp::new("workspace-command-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource(PendingWorkspaceControls::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_systems(LayoutSchedule, workspace_command_system);

        let fallback_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        let target_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false })
            .id();
        let window_entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 55 },
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 800, height: 600 },
                    scene_geometry: nekoland_ecs::components::WindowSceneGeometry {
                        x: 0,
                        y: 0,
                        width: 800,
                        height: 600,
                    },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: true, scale: 1 },
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    management_hints: nekoland_ecs::components::WindowManagementHints::native_wayland(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    decoration: ServerDecoration::default(),
                    border_theme: BorderTheme::default(),
                    animation: WindowAnimation::default(),
                },
                ChildOf(target_workspace),
            ))
            .id();

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWorkspaceControls>()
            .destroy_id(WorkspaceId(2));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert!(
            world.get_entity(target_workspace).is_err(),
            "destroyed workspace entity should be removed",
        );

        let Some(child_of) = world.get::<ChildOf>(window_entity) else {
            panic!("window should keep ChildOf");
        };
        assert_eq!(
            child_of.parent(),
            fallback_workspace,
            "window should reparent to the fallback workspace entity",
        );
    }

    #[test]
    fn sync_workspace_disabled_state_recursively_disables_inactive_workspaces() {
        let mut app = NekolandApp::new("workspace-disabled-test");
        app.inner_mut().add_systems(LayoutSchedule, sync_workspace_disabled_state_system);

        let active_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        let inactive_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false })
            .id();
        let inactive_window = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 77 },
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 640, height: 480 },
                    scene_geometry: nekoland_ecs::components::WindowSceneGeometry {
                        x: 0,
                        y: 0,
                        width: 640,
                        height: 480,
                    },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: true, scale: 1 },
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    management_hints: nekoland_ecs::components::WindowManagementHints::native_wayland(),
                    layout: WindowLayout::Tiled,
                    mode: WindowMode::Normal,
                    decoration: ServerDecoration::default(),
                    border_theme: BorderTheme::default(),
                    animation: WindowAnimation::default(),
                },
                ChildOf(inactive_workspace),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert!(
            world.get::<Disabled>(inactive_workspace).is_some(),
            "inactive workspace root should be disabled",
        );
        assert!(
            world.get::<Disabled>(inactive_window).is_some(),
            "inactive workspace children should be disabled recursively",
        );
        assert!(
            world.get::<Disabled>(active_workspace).is_none(),
            "active workspace should remain enabled",
        );
    }

    #[test]
    fn sync_active_workspace_marker_tracks_current_active_workspace() {
        let mut app = NekolandApp::new("workspace-active-marker-test");
        app.inner_mut()
            .insert_resource(FocusedOutputState::default())
            .insert_resource(WaylandIngress::default())
            .add_systems(LayoutSchedule, sync_active_workspace_marker_system);

        let active_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        let inactive_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false })
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert!(world.get::<ActiveWorkspace>(active_workspace).is_some());
        assert!(world.get::<ActiveWorkspace>(inactive_workspace).is_none());
    }

    #[test]
    fn sync_active_workspace_marker_clears_stale_active_flag_on_disabled_workspace() {
        let mut app = NekolandApp::new("workspace-active-disabled-test");
        app.inner_mut()
            .insert_resource(FocusedOutputState::default())
            .insert_resource(WaylandIngress::default())
            .add_systems(LayoutSchedule, sync_active_workspace_marker_system);

        let stale_active_workspace = app
            .inner_mut()
            .world_mut()
            .spawn((
                Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true },
                Disabled,
                ActiveWorkspace,
            ))
            .id();
        let preferred_workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(2), name: "2".to_owned(), active: false })
            .id();
        app.inner_mut().world_mut().spawn((
            OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "test".to_owned(),
                    model: "one".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            },
            OutputCurrentWorkspace { workspace: WorkspaceId(2) },
        ));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let Some(stale_workspace) = world.get::<Workspace>(stale_active_workspace) else {
            panic!("stale workspace should remain present");
        };
        assert!(!stale_workspace.active);
        assert!(world.get::<ActiveWorkspace>(stale_active_workspace).is_none());
        let Some(preferred_workspace_state) = world.get::<Workspace>(preferred_workspace) else {
            panic!("preferred workspace should remain present");
        };
        assert!(preferred_workspace_state.active);
        assert!(world.get::<ActiveWorkspace>(preferred_workspace).is_some());
    }

    #[test]
    fn workspace_reconciliation_condition_stops_ticking_when_state_is_stable() {
        let mut app = NekolandApp::new("workspace-reconciliation-condition-test");
        app.insert_resource(PendingWorkspaceControls::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(WaylandIngress::default())
            .insert_resource(WorkspaceReconciliationAudit::default());
        app.inner_mut().add_systems(
            LayoutSchedule,
            record_workspace_reconciliation.run_if(workspace_reconciliation_needed),
        );

        app.inner_mut().world_mut().spawn((
            Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true },
            ActiveWorkspace,
        ));
        let output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "Virtual-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "one".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1280,
                        height: 720,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(1) },
            ))
            .id();
        let output_id =
            *app.inner().world().get::<OutputId>(output).expect("output id should exist");
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(output_id);

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        {
            let Some(audit) = app.inner().world().get_resource::<WorkspaceReconciliationAudit>()
            else {
                panic!("workspace reconciliation audit should exist");
            };
            assert_eq!(
                audit.0, 1,
                "stable workspace routing should stop re-triggering the condition"
            );
        }

        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id =
            Some(OutputId(u64::MAX));
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(audit) = app.inner().world().get_resource::<WorkspaceReconciliationAudit>() else {
            panic!("workspace reconciliation audit should still exist");
        };
        assert_eq!(audit.0, 2, "focused-output changes should re-trigger reconciliation once");
    }

    #[test]
    fn output_workspace_housekeeping_assigns_unique_workspaces_to_outputs() {
        let mut app = NekolandApp::new("output-workspace-housekeeping-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource(RememberedOutputWorkspaceState::default())
            .inner_mut()
            .add_systems(LayoutSchedule, output_workspace_housekeeping_system);

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(2),
            name: "2".to_owned(),
            active: false,
        });
        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "one".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "HDMI-A-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "two".to_owned(),
            },
            properties: OutputProperties {
                width: 1920,
                height: 1080,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut outputs = world.query::<(&OutputDevice, &OutputCurrentWorkspace)>();
        let assignments = outputs
            .iter(world)
            .map(|(output, current_workspace)| (output.name.clone(), current_workspace.workspace.0))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(assignments.len(), 2);
        assert_ne!(assignments["Virtual-1"], assignments["HDMI-A-1"]);
    }

    #[test]
    fn workspace_switch_targets_focused_output_workspace() {
        let mut app = NekolandApp::new("workspace-switch-focused-output-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource(RememberedOutputWorkspaceState::default())
            .insert_resource(PendingWorkspaceControls::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(WaylandIngress::default())
            .inner_mut()
            .add_systems(
                LayoutSchedule,
                (
                    workspace_command_system,
                    output_workspace_housekeeping_system,
                    remember_output_workspace_routes_system,
                )
                    .chain(),
            );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(2),
            name: "2".to_owned(),
            active: false,
        });
        let virtual_output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "Virtual-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "one".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1280,
                        height: 720,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(1) },
            ))
            .id();
        let hdmi_output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "HDMI-A-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "two".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1920,
                        height: 1080,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(2) },
            ))
            .id();
        let _virtual_output_id = *app
            .inner()
            .world()
            .get::<OutputId>(virtual_output)
            .expect("virtual output id should exist");
        let hdmi_output_id =
            *app.inner().world().get::<OutputId>(hdmi_output).expect("hdmi output id should exist");
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(hdmi_output_id);

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWorkspaceControls>()
            .switch_or_create_named("3");

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut outputs = world.query::<(&OutputDevice, &OutputCurrentWorkspace)>();
        let assignments = outputs
            .iter(world)
            .map(|(output, current_workspace)| (output.name.clone(), current_workspace.workspace.0))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(assignments["Virtual-1"], 1);
        assert_eq!(assignments["HDMI-A-1"], 3);
    }

    #[test]
    fn workspace_switch_prefers_wayland_ingress_primary_output_over_stale_compat_state() {
        let mut app = NekolandApp::new("workspace-switch-wayland-primary-output-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource(RememberedOutputWorkspaceState::default())
            .insert_resource(PendingWorkspaceControls::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(PrimaryOutputState::default())
            .inner_mut()
            .add_systems(
                LayoutSchedule,
                (
                    workspace_command_system,
                    output_workspace_housekeeping_system,
                    remember_output_workspace_routes_system,
                )
                    .chain(),
            );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(2),
            name: "2".to_owned(),
            active: false,
        });
        let virtual_output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "Virtual-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "one".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1280,
                        height: 720,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(1) },
            ))
            .id();
        let hdmi_output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "HDMI-A-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "two".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1920,
                        height: 1080,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(2) },
            ))
            .id();
        let virtual_output_id = *app
            .inner()
            .world()
            .get::<OutputId>(virtual_output)
            .expect("virtual output id should exist");
        let hdmi_output_id =
            *app.inner().world().get::<OutputId>(hdmi_output).expect("hdmi output id should exist");
        // Keep the stale compat primary-output resource on purpose so boundary routing must win.
        app.inner_mut().world_mut().resource_mut::<PrimaryOutputState>().id =
            Some(virtual_output_id);
        app.inner_mut().world_mut().insert_resource(WaylandIngress {
            primary_output: PrimaryOutputState { id: Some(hdmi_output_id) },
            ..WaylandIngress::default()
        });

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingWorkspaceControls>()
            .switch_or_create_named("3");

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut outputs = world.query::<(&OutputDevice, &OutputCurrentWorkspace)>();
        let assignments = outputs
            .iter(world)
            .map(|(output, current_workspace)| (output.name.clone(), current_workspace.workspace.0))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(assignments["Virtual-1"], 1);
        assert_eq!(assignments["HDMI-A-1"], 3);
    }

    #[test]
    fn output_workspace_housekeeping_restores_remembered_workspace_for_reconnected_output() {
        let mut app = NekolandApp::new("output-workspace-reconnect-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource({
                let mut remembered = RememberedOutputWorkspaceState::default();
                remembered.ids_by_name.insert("Virtual-1".to_owned(), OutputId(1));
                remembered.by_id.insert(OutputId(1), WorkspaceId(2));
                remembered
            })
            .inner_mut()
            .add_systems(LayoutSchedule, output_workspace_housekeeping_system);

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(2),
            name: "2".to_owned(),
            active: false,
        });
        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "reconnected".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let assignment = world
            .query::<(&OutputDevice, &OutputCurrentWorkspace)>()
            .iter(world)
            .find(|(output, _)| output.name == "Virtual-1")
            .map(|(_, current_workspace)| current_workspace.workspace.0);
        let Some(assignment) = assignment else {
            panic!("reconnected output should receive a workspace assignment");
        };
        assert_eq!(assignment, 2);
    }
}
