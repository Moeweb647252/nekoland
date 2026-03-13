use std::collections::BTreeMap;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Resource};
use bevy_ecs::query::Allow;

use crate::components::{OutputDevice, WlSurfaceHandle, Workspace};

/// Indexes runtime entities by stable external identifiers.
///
/// This is a transitional resource used to migrate from "manual joins" on ids/names
/// to relationship-driven ECS logic, without changing protocol / IPC boundaries.
#[derive(Debug, Default, Resource)]
pub struct EntityIndex {
    surface_to_entity: BTreeMap<u64, Entity>,
    workspace_id_to_entity: BTreeMap<u32, Entity>,
    workspace_name_to_entity: BTreeMap<String, Entity>,
    output_name_to_entity: BTreeMap<String, Entity>,
}

impl EntityIndex {
    /// Clears every cached lookup table before the index is rebuilt from the world.
    pub fn clear(&mut self) {
        self.surface_to_entity.clear();
        self.workspace_id_to_entity.clear();
        self.workspace_name_to_entity.clear();
        self.output_name_to_entity.clear();
    }

    /// Looks up an entity by stable surface id.
    pub fn entity_for_surface(&self, surface_id: u64) -> Option<Entity> {
        self.surface_to_entity.get(&surface_id).copied()
    }

    /// Looks up a workspace entity by numeric workspace id.
    pub fn entity_for_workspace_id(&self, workspace_id: u32) -> Option<Entity> {
        self.workspace_id_to_entity.get(&workspace_id).copied()
    }

    /// Looks up a workspace entity by display name.
    pub fn entity_for_workspace_name(&self, workspace_name: &str) -> Option<Entity> {
        self.workspace_name_to_entity.get(workspace_name).copied()
    }

    /// Looks up an output entity by output name.
    pub fn entity_for_output_name(&self, output_name: &str) -> Option<Entity> {
        self.output_name_to_entity.get(output_name).copied()
    }

    /// Inserts or refreshes the cached workspace lookup entries for one entity.
    pub fn insert_workspace(
        &mut self,
        workspace_entity: Entity,
        workspace_id: u32,
        workspace_name: String,
    ) {
        self.workspace_id_to_entity.insert(workspace_id, workspace_entity);
        self.workspace_name_to_entity.insert(workspace_name, workspace_entity);
    }

    /// Removes every workspace lookup that points at the removed entity.
    pub fn remove_workspace_entity(&mut self, workspace_entity: Entity) {
        self.workspace_id_to_entity.retain(|_, entity| *entity != workspace_entity);
        self.workspace_name_to_entity.retain(|_, entity| *entity != workspace_entity);
    }

    /// Reverse lookup used when relationships refer to an entity but IPC needs a surface id.
    pub fn surface_id_for_entity(&self, surface_entity: Entity) -> Option<u64> {
        self.surface_to_entity
            .iter()
            .find_map(|(id, entity)| (*entity == surface_entity).then_some(*id))
    }
}

/// Rebuilds the transitional entity index from the current world snapshot.
pub fn rebuild_entity_index_system(
    mut index: bevy_ecs::prelude::ResMut<EntityIndex>,
    surfaces: Query<(Entity, &WlSurfaceHandle), Allow<Disabled>>,
    workspaces: Query<(Entity, &Workspace), Allow<Disabled>>,
    outputs: Query<(Entity, &OutputDevice)>,
) {
    index.clear();

    for (entity, surface) in &surfaces {
        index.surface_to_entity.insert(surface.id, entity);
    }

    for (entity, workspace) in &workspaces {
        index.workspace_id_to_entity.insert(workspace.id.0, entity);
        index.workspace_name_to_entity.insert(workspace.name.clone(), entity);
    }

    for (entity, output) in &outputs {
        index.output_name_to_entity.insert(output.name.clone(), entity);
    }
}
