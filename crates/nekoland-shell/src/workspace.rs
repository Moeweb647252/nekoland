use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::{Commands, Entity, Has, Query, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{ActiveWorkspace, Workspace, WorkspaceId, XdgWindow};
use nekoland_ecs::resources::{EntityIndex, PendingWorkspaceControls, WorkspaceControl};
use nekoland_ecs::selectors::{WorkspaceLookup, WorkspaceName, WorkspaceSelector};
use nekoland_ecs::views::WorkspaceRuntime;
use nekoland_ecs::workspace_membership::window_in_workspace;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceManager;

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

/// Applies create/switch/destroy requests and keeps child window relationships in sync when a
/// workspace disappears.
pub fn workspace_command_system(
    mut commands: Commands,
    mut pending_workspace_controls: ResMut<PendingWorkspaceControls>,
    mut entity_index: ResMut<EntityIndex>,
    mut workspaces: Query<(Entity, WorkspaceRuntime), Allow<Disabled>>,
    windows: Query<(Entity, Option<&ChildOf>), (With<XdgWindow>, Allow<Disabled>)>,
) {
    let mut snapshot = workspaces
        .iter_mut()
        .map(|(entity, workspace)| {
            (entity, workspace.id().0, workspace.name().to_owned(), workspace.is_active())
        })
        .collect::<Vec<_>>();

    for control in pending_workspace_controls.take() {
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
                entity_index.insert_workspace(
                    workspace_entity,
                    workspace_id,
                    workspace_name.clone(),
                );
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
                        entity_index.insert_workspace(
                            workspace_entity,
                            workspace_id,
                            workspace_name.clone(),
                        );
                        (workspace_entity, workspace_id, workspace_name)
                    });

                for (entity, mut active_workspace) in &mut workspaces {
                    active_workspace.workspace.active = entity == target_entity;
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

                for (window_entity, child_of) in &windows {
                    if !window_in_workspace(child_of, target_entity) {
                        continue;
                    }

                    if let Some(fallback_entity) = fallback_entity {
                        commands.entity(window_entity).insert(ChildOf(fallback_entity));
                    }
                }

                for (_, mut existing_workspace) in &mut workspaces {
                    if existing_workspace.id().0 == target_id {
                        continue;
                    }
                    existing_workspace.workspace.active = existing_workspace.id().0 == fallback_id;
                }

                entity_index.remove_workspace_entity(target_entity);
                commands.entity(target_entity).despawn();

                snapshot.retain(|(_, id, _, _)| *id != target_id);
                for entry in &mut snapshot {
                    entry.3 = entry.1 == fallback_id;
                }
            }
        }
    }
}

/// Mirrors the active workspace selection into Bevy's `Disabled` hierarchy state so downstream
/// systems can ignore inactive workspaces without filtering every query manually.
pub fn sync_active_workspace_marker_system(
    mut commands: Commands,
    workspaces: Query<(Entity, WorkspaceRuntime, Has<ActiveWorkspace>)>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|(_, workspace, _)| workspace.is_active())
        .map(|(entity, _, _)| entity)
        .or_else(|| {
            workspaces
                .iter()
                .min_by_key(|(_, workspace, _)| workspace.id().0)
                .map(|(entity, _, _)| entity)
        });

    for (entity, _, has_active_marker) in &workspaces {
        if Some(entity) == active_workspace {
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
    workspaces: Query<(Entity, WorkspaceRuntime, Has<Disabled>)>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|(_, workspace, _)| workspace.is_active())
        .map(|(entity, _, _)| entity)
        .or_else(|| {
            workspaces
                .iter()
                .min_by_key(|(_, workspace, _)| workspace.id().0)
                .map(|(entity, _, _)| entity)
        });

    for (entity, _, is_disabled) in &workspaces {
        if Some(entity) == active_workspace {
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

#[cfg(test)]
mod tests {
    use bevy_ecs::entity_disabling::Disabled;
    use bevy_ecs::hierarchy::ChildOf;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        ActiveWorkspace, BorderTheme, BufferState, ServerDecoration, SurfaceGeometry,
        WindowAnimation, WindowLayout, WindowMode, WlSurfaceHandle, Workspace, WorkspaceId,
        XdgWindow,
    };
    use nekoland_ecs::resources::{EntityIndex, PendingWorkspaceControls};

    use super::{
        sync_active_workspace_marker_system, sync_workspace_disabled_state_system,
        workspace_command_system,
    };

    #[test]
    fn destroying_workspace_reparents_windows_to_fallback_workspace() {
        let mut app = NekolandApp::new("workspace-command-test");
        app.insert_resource(EntityIndex::default())
            .insert_resource(PendingWorkspaceControls::default());
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
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
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

        let child_of = world.get::<ChildOf>(window_entity).expect("window should keep ChildOf");
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
                    buffer: BufferState { attached: true, scale: 1 },
                    window: XdgWindow::default(),
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
        app.inner_mut().add_systems(LayoutSchedule, sync_active_workspace_marker_system);

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
}
