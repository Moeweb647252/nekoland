//! Helpers for resolving workspace/output relationships from shared runtime views.

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Query};
use bevy_ecs::query::QueryFilter;

use crate::components::{OutputId, Workspace};
use crate::resources::EntityIndex;
use crate::views::{OutputRuntime, WorkspaceRuntime};

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

/// Resolves the output name currently hosting the given workspace id.
pub fn output_name_for_workspace_runtime_id(
    workspace_id: Option<u32>,
    outputs: &Query<(Entity, OutputRuntime), impl QueryFilter>,
) -> Option<String> {
    let workspace_id = workspace_id?;
    outputs.iter().find_map(|(_, output)| {
        output
            .current_workspace
            .as_ref()
            .is_some_and(|current_workspace| current_workspace.workspace.0 == workspace_id)
            .then(|| output.name().to_owned())
    })
}

/// Resolves an output name from a stable output id.
pub fn output_name_for_output_id(
    output_id: OutputId,
    outputs: &Query<(Entity, OutputRuntime), impl QueryFilter>,
) -> Option<String> {
    outputs
        .iter()
        .find(|(_, output)| output.id() == output_id)
        .map(|(_, output)| output.name().to_owned())
}

/// Returns the focused output name, then the primary output name, then the first available output.
pub fn focused_or_primary_output_name(
    outputs: &Query<(Entity, OutputRuntime), impl QueryFilter>,
    focused_output_id: Option<OutputId>,
    primary_output_id: Option<OutputId>,
) -> Option<String> {
    if let Some(output_id) = focused_output_id
        && let Some(output_name) = output_name_for_output_id(output_id, outputs)
    {
        return Some(output_name);
    }

    if let Some(output_id) = primary_output_id
        && let Some(output_name) = output_name_for_output_id(output_id, outputs)
    {
        return Some(output_name);
    }

    outputs.iter().next().map(|(_, output)| output.name().to_owned())
}

/// Resolves the current workspace entity associated with the named output.
pub fn current_workspace_runtime_target_for_output_name(
    output_name: &str,
    outputs: &Query<(Entity, OutputRuntime), impl QueryFilter>,
    entity_index: &EntityIndex,
    fallback_workspace_id: u32,
) -> Option<Entity> {
    outputs
        .iter()
        .find(|(_, output)| output.name() == output_name)
        .and_then(|(_, output)| {
            output.current_workspace.as_ref().and_then(|current_workspace| {
                entity_index.entity_for_workspace_id(current_workspace.workspace.0)
            })
        })
        .or_else(|| entity_index.entity_for_workspace_id(fallback_workspace_id))
}

/// Resolves the current workspace entity associated with the given output id.
pub fn current_workspace_runtime_target_for_output_id(
    output_id: OutputId,
    outputs: &Query<(Entity, OutputRuntime), impl QueryFilter>,
    entity_index: &EntityIndex,
    fallback_workspace_id: u32,
) -> Option<Entity> {
    outputs
        .iter()
        .find(|(_, output)| output.id() == output_id)
        .and_then(|(_, output)| {
            output.current_workspace.as_ref().and_then(|current_workspace| {
                entity_index.entity_for_workspace_id(current_workspace.workspace.0)
            })
        })
        .or_else(|| entity_index.entity_for_workspace_id(fallback_workspace_id))
}

/// Resolves the focused output's workspace target, falling back to primary or a configured default.
pub fn focused_or_primary_workspace_runtime_target(
    outputs: &Query<(Entity, OutputRuntime), impl QueryFilter>,
    focused_output_id: Option<OutputId>,
    primary_output_id: Option<OutputId>,
    entity_index: &EntityIndex,
    fallback_workspace_id: u32,
) -> Option<Entity> {
    if let Some(output_id) = focused_output_id {
        return current_workspace_runtime_target_for_output_id(
            output_id,
            outputs,
            entity_index,
            fallback_workspace_id,
        );
    }

    if let Some(output_id) = primary_output_id {
        return current_workspace_runtime_target_for_output_id(
            output_id,
            outputs,
            entity_index,
            fallback_workspace_id,
        );
    }

    focused_or_primary_output_name(outputs, focused_output_id, primary_output_id)
        .and_then(|output_name| {
            current_workspace_runtime_target_for_output_name(
                &output_name,
                outputs,
                entity_index,
                fallback_workspace_id,
            )
        })
        .or_else(|| entity_index.entity_for_workspace_id(fallback_workspace_id))
}
