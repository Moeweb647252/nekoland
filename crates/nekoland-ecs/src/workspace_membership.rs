use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Query};
use bevy_ecs::query::QueryFilter;

use crate::components::Workspace;
use crate::resources::EntityIndex;
use crate::views::WorkspaceRuntime;

/// Returns the active workspace entity and id, falling back to the lowest workspace id when no
/// workspace is currently marked active.
pub fn active_workspace_target(
    workspaces: &Query<(Entity, &Workspace), impl QueryFilter>,
) -> (Option<Entity>, Option<u32>) {
    workspaces
        .iter()
        .find(|(_, workspace)| workspace.active)
        .map(|(entity, workspace)| (Some(entity), Some(workspace.id.0)))
        .or_else(|| {
            workspaces
                .iter()
                .min_by_key(|(_, workspace)| workspace.id)
                .map(|(entity, workspace)| (Some(entity), Some(workspace.id.0)))
        })
        .unwrap_or((None, None))
}

/// Same as [`active_workspace_target`], but over the shared `WorkspaceRuntime` view.
pub fn active_workspace_runtime_target(
    workspaces: &Query<(Entity, WorkspaceRuntime), impl QueryFilter>,
) -> (Option<Entity>, Option<u32>) {
    workspaces
        .iter()
        .find(|(_, workspace)| workspace.is_active())
        .map(|(entity, workspace)| (Some(entity), Some(workspace.id().0)))
        .or_else(|| {
            workspaces
                .iter()
                .min_by_key(|(_, workspace)| workspace.id())
                .map(|(entity, workspace)| (Some(entity), Some(workspace.id().0)))
        })
        .unwrap_or((None, None))
}

/// Returns the active workspace entity or falls back to a workspace id lookup through the index.
pub fn active_workspace_target_or_index(
    workspaces: &Query<(Entity, &Workspace), impl QueryFilter>,
    entity_index: &EntityIndex,
    fallback_workspace_id: u32,
) -> Option<Entity> {
    match active_workspace_target(workspaces) {
        (Some(entity), Some(_)) => Some(entity),
        _ => entity_index.entity_for_workspace_id(fallback_workspace_id),
    }
}

/// Same as [`active_workspace_target_or_index`], but over the shared `WorkspaceRuntime` view.
pub fn active_workspace_runtime_target_or_index(
    workspaces: &Query<(Entity, WorkspaceRuntime), impl QueryFilter>,
    entity_index: &EntityIndex,
    fallback_workspace_id: u32,
) -> Option<Entity> {
    match active_workspace_runtime_target(workspaces) {
        (Some(entity), Some(_)) => Some(entity),
        _ => entity_index.entity_for_workspace_id(fallback_workspace_id),
    }
}

/// Checks whether a window/popup entity is currently parented to the given workspace entity.
pub fn window_in_workspace(child_of: Option<&ChildOf>, workspace_entity: Entity) -> bool {
    child_of.is_some_and(|child_of| child_of.parent() == workspace_entity)
}

/// Resolves the numeric workspace id for an entity via its `ChildOf` workspace relationship.
pub fn window_workspace_id(
    child_of: Option<&ChildOf>,
    workspaces: &Query<(Entity, &Workspace), impl QueryFilter>,
) -> Option<u32> {
    child_of.and_then(|child_of| {
        workspaces.get(child_of.parent()).ok().map(|(_, workspace)| workspace.id.0)
    })
}

/// Same as [`window_workspace_id`], but over the shared `WorkspaceRuntime` view.
pub fn window_workspace_runtime_id(
    child_of: Option<&ChildOf>,
    workspaces: &Query<(Entity, WorkspaceRuntime), impl QueryFilter>,
) -> Option<u32> {
    child_of.and_then(|child_of| {
        workspaces.get(child_of.parent()).ok().map(|(_, workspace)| workspace.id().0)
    })
}
