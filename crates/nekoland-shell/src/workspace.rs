use bevy_ecs::prelude::{Commands, Entity, Query, ResMut, With};
use nekoland_ecs::components::{LayoutSlot, Workspace, WorkspaceId, XdgWindow};
use nekoland_ecs::resources::{PendingWorkspaceServerRequests, WorkspaceServerAction};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceManager;

pub fn workspace_switch_system(mut commands: Commands, mut workspaces: Query<&mut Workspace>) {
    if workspaces.is_empty() {
        commands.spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true });
        tracing::trace!("seeded initial workspace");
        return;
    }

    if workspaces.iter().all(|workspace| !workspace.active) {
        let fallback_id = workspaces.iter().map(|workspace| workspace.id.0).min().unwrap_or(1);
        for mut workspace in &mut workspaces {
            workspace.active = workspace.id.0 == fallback_id;
        }
    }

    tracing::trace!(count = workspaces.iter().count(), "workspace housekeeping tick");
}

pub fn workspace_command_system(
    mut commands: Commands,
    mut pending_workspace_requests: ResMut<PendingWorkspaceServerRequests>,
    mut workspaces: Query<(Entity, &mut Workspace)>,
    mut layout_slots: Query<&mut LayoutSlot, With<XdgWindow>>,
) {
    let mut snapshot = workspaces
        .iter_mut()
        .map(|(entity, workspace)| {
            (entity, workspace.id.0, workspace.name.clone(), workspace.active)
        })
        .collect::<Vec<_>>();

    for request in pending_workspace_requests.items.drain(..) {
        match request.action {
            WorkspaceServerAction::Create { workspace } => {
                if snapshot
                    .iter()
                    .any(|(_, id, name, _)| *name == workspace || id.to_string() == workspace)
                {
                    continue;
                }

                let workspace_id = workspace
                    .parse::<u32>()
                    .ok()
                    .filter(|candidate| snapshot.iter().all(|(_, id, _, _)| id != candidate))
                    .unwrap_or_else(|| next_workspace_id(&snapshot));
                let workspace_name = workspace.clone();
                commands.spawn(Workspace {
                    id: WorkspaceId(workspace_id),
                    name: workspace_name.clone(),
                    active: false,
                });
                snapshot.push((Entity::PLACEHOLDER, workspace_id, workspace_name, false));
            }
            WorkspaceServerAction::Switch { workspace } => {
                let target = snapshot
                    .iter()
                    .find(|(_, id, name, _)| *name == workspace || id.to_string() == workspace)
                    .map(|(entity, id, name, _)| (*entity, *id, name.clone()));
                let (target_entity, target_id, target_name) = target.unwrap_or_else(|| {
                    let workspace_id =
                        workspace.parse::<u32>().unwrap_or_else(|_| next_workspace_id(&snapshot));
                    commands.spawn(Workspace {
                        id: WorkspaceId(workspace_id),
                        name: workspace.clone(),
                        active: true,
                    });
                    (Entity::PLACEHOLDER, workspace_id, workspace.clone())
                });

                if target_entity != Entity::PLACEHOLDER {
                    for (entity, mut active_workspace) in &mut workspaces {
                        active_workspace.active = entity == target_entity;
                    }
                } else {
                    for (_, mut active_workspace) in &mut workspaces {
                        active_workspace.active = false;
                    }
                }

                snapshot.retain(|(entity, _, _, _)| *entity != target_entity);
                snapshot.push((target_entity, target_id, target_name, true));
                for entry in &mut snapshot {
                    entry.3 = entry.1 == target_id;
                }
            }
            WorkspaceServerAction::Destroy { workspace } => {
                if snapshot.len() <= 1 {
                    continue;
                }

                let Some((target_entity, target_id, _, target_active)) = snapshot
                    .iter()
                    .find(|(_, id, name, _)| *name == workspace || id.to_string() == workspace)
                    .cloned()
                else {
                    continue;
                };

                let fallback_id = snapshot
                    .iter()
                    .filter(|(_, id, _, _)| *id != target_id)
                    .find(|(_, _, _, active)| *active || target_active)
                    .map(|(_, id, _, _)| *id)
                    .unwrap_or_else(|| {
                        snapshot
                            .iter()
                            .find(|(_, id, _, _)| *id != target_id)
                            .map(|(_, id, _, _)| *id)
                            .unwrap_or(1)
                    });

                if target_entity != Entity::PLACEHOLDER {
                    commands.entity(target_entity).despawn();
                }

                for (_, mut existing_workspace) in &mut workspaces {
                    if existing_workspace.id.0 == target_id {
                        continue;
                    }
                    existing_workspace.active = existing_workspace.id.0 == fallback_id;
                }

                for mut layout_slot in &mut layout_slots {
                    if layout_slot.workspace == target_id {
                        layout_slot.workspace = fallback_id;
                    }
                }

                snapshot.retain(|(_, id, _, _)| *id != target_id);
                for entry in &mut snapshot {
                    entry.3 = entry.1 == fallback_id;
                }
            }
        }
    }
}

fn next_workspace_id(snapshot: &[(Entity, u32, String, bool)]) -> u32 {
    snapshot.iter().map(|(_, id, _, _)| *id).max().unwrap_or(0).saturating_add(1)
}
