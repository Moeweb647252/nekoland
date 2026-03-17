use std::collections::{BTreeMap, HashMap};

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::{Entity, Query, Resource};
use bevy_ecs::query::Allow;
use bevy_ecs::world::{DeferredWorld, World};

use crate::components::{OutputDevice, WlSurfaceHandle, Workspace};

/// Indexes runtime entities by stable external identifiers.
///
/// This is a transitional resource used to migrate from "manual joins" on ids/names
/// to relationship-driven ECS logic, without changing protocol / IPC boundaries.
#[derive(Debug, Default, Resource)]
pub struct EntityIndex {
    surface_to_entity: BTreeMap<u64, Entity>,
    entity_to_surface: HashMap<Entity, u64>,
    workspace_id_to_entity: BTreeMap<u32, Entity>,
    workspace_name_to_entity: BTreeMap<String, Entity>,
    output_name_to_entity: BTreeMap<String, Entity>,
}

impl EntityIndex {
    /// Clears every cached lookup table before the index is rebuilt from the world.
    pub fn clear(&mut self) {
        self.surface_to_entity.clear();
        self.entity_to_surface.clear();
        self.workspace_id_to_entity.clear();
        self.workspace_name_to_entity.clear();
        self.output_name_to_entity.clear();
    }

    /// Inserts or refreshes the cached surface lookup entries for one entity.
    pub fn insert_surface(&mut self, surface_entity: Entity, surface_id: u64) {
        if let Some(previous_surface_id) = self.entity_to_surface.insert(surface_entity, surface_id)
        {
            self.surface_to_entity.remove(&previous_surface_id);
        }
        self.surface_to_entity.insert(surface_id, surface_entity);
    }

    /// Looks up an entity by stable surface id.
    pub fn entity_for_surface(&self, surface_id: u64) -> Option<Entity> {
        self.surface_to_entity.get(&surface_id).copied()
    }

    /// Removes every surface lookup that points at the removed entity.
    pub fn remove_surface_entity(&mut self, surface_entity: Entity) {
        if let Some(surface_id) = self.entity_to_surface.remove(&surface_entity) {
            self.surface_to_entity.remove(&surface_id);
        }
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
        self.entity_to_surface.get(&surface_entity).copied()
    }

    /// Inserts or refreshes the cached output lookup entry for one entity.
    pub fn insert_output(&mut self, output_entity: Entity, output_name: String) {
        self.output_name_to_entity.insert(output_name, output_entity);
    }

    /// Removes every output lookup that points at the removed entity.
    pub fn remove_output_entity(&mut self, output_entity: Entity) {
        self.output_name_to_entity.retain(|_, entity| *entity != output_entity);
    }
}

/// Installs component hooks that keep the transitional entity index in sync incrementally.
pub fn register_entity_index_hooks(world: &mut World) {
    world.init_resource::<EntityIndex>();

    let surface_hooks = world.register_component_hooks::<WlSurfaceHandle>();
    surface_hooks.try_on_insert(index_surface_handle_inserted);
    surface_hooks.try_on_replace(index_surface_handle_removed);
    surface_hooks.try_on_remove(index_surface_handle_removed);

    let workspace_hooks = world.register_component_hooks::<Workspace>();
    workspace_hooks.try_on_insert(index_workspace_inserted);
    workspace_hooks.try_on_replace(index_workspace_removed);
    workspace_hooks.try_on_remove(index_workspace_removed);

    let output_hooks = world.register_component_hooks::<OutputDevice>();
    output_hooks.try_on_insert(index_output_inserted);
    output_hooks.try_on_replace(index_output_removed);
    output_hooks.try_on_remove(index_output_removed);
}

fn index_surface_handle_inserted(mut world: DeferredWorld, context: HookContext) {
    let Some(surface_id) = world.get::<WlSurfaceHandle>(context.entity).map(|surface| surface.id)
    else {
        return;
    };

    world.resource_mut::<EntityIndex>().insert_surface(context.entity, surface_id);
}

fn index_surface_handle_removed(mut world: DeferredWorld, context: HookContext) {
    world.resource_mut::<EntityIndex>().remove_surface_entity(context.entity);
}

fn index_workspace_inserted(mut world: DeferredWorld, context: HookContext) {
    let Some((workspace_id, workspace_name)) = world
        .get::<Workspace>(context.entity)
        .map(|workspace| (workspace.id.0, workspace.name.clone()))
    else {
        return;
    };

    world.resource_mut::<EntityIndex>().insert_workspace(
        context.entity,
        workspace_id,
        workspace_name,
    );
}

fn index_workspace_removed(mut world: DeferredWorld, context: HookContext) {
    world.resource_mut::<EntityIndex>().remove_workspace_entity(context.entity);
}

fn index_output_inserted(mut world: DeferredWorld, context: HookContext) {
    let Some(output_name) =
        world.get::<OutputDevice>(context.entity).map(|output| output.name.clone())
    else {
        return;
    };

    world.resource_mut::<EntityIndex>().insert_output(context.entity, output_name);
}

fn index_output_removed(mut world: DeferredWorld, context: HookContext) {
    world.resource_mut::<EntityIndex>().remove_output_entity(context.entity);
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
        index.insert_surface(entity, surface.id);
    }

    for (entity, workspace) in &workspaces {
        index.insert_workspace(entity, workspace.id.0, workspace.name.clone());
    }

    for (entity, output) in &outputs {
        index.insert_output(entity, output.name.clone());
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::{EntityIndex, register_entity_index_hooks};
    use crate::components::{OutputDevice, OutputKind, WlSurfaceHandle, Workspace, WorkspaceId};

    #[test]
    fn component_hooks_track_entity_index_incrementally() {
        let mut world = World::new();
        register_entity_index_hooks(&mut world);

        let surface = world.spawn(WlSurfaceHandle { id: 11 }).id();
        let workspace =
            world.spawn(Workspace { id: WorkspaceId(7), name: "7".to_owned(), active: true }).id();
        let output = world
            .spawn(OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "test".to_owned(),
            })
            .id();

        let index = world.resource::<EntityIndex>();
        assert_eq!(index.entity_for_surface(11), Some(surface));
        assert_eq!(index.surface_id_for_entity(surface), Some(11));
        assert_eq!(index.entity_for_workspace_id(7), Some(workspace));
        assert_eq!(index.entity_for_workspace_name("7"), Some(workspace));
        assert_eq!(index.entity_for_output_name("Virtual-1"), Some(output));
    }

    #[test]
    fn component_hooks_refresh_entity_index_on_replace_and_remove() {
        let mut world = World::new();
        register_entity_index_hooks(&mut world);

        let workspace =
            world.spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true }).id();
        let output = world
            .spawn(OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "test".to_owned(),
            })
            .id();

        world.entity_mut(workspace).insert(Workspace {
            id: WorkspaceId(2),
            name: "2".to_owned(),
            active: false,
        });
        world.entity_mut(output).insert(OutputDevice {
            name: "HDMI-A-1".to_owned(),
            kind: OutputKind::Virtual,
            make: "test".to_owned(),
            model: "test".to_owned(),
        });

        {
            let index = world.resource::<EntityIndex>();
            assert_eq!(index.entity_for_workspace_id(1), None);
            assert_eq!(index.entity_for_workspace_name("1"), None);
            assert_eq!(index.entity_for_workspace_id(2), Some(workspace));
            assert_eq!(index.entity_for_workspace_name("2"), Some(workspace));
            assert_eq!(index.entity_for_output_name("Virtual-1"), None);
            assert_eq!(index.entity_for_output_name("HDMI-A-1"), Some(output));
        }

        world.despawn(workspace);
        world.despawn(output);

        let index = world.resource::<EntityIndex>();
        assert_eq!(index.entity_for_workspace_id(2), None);
        assert_eq!(index.entity_for_workspace_name("2"), None);
        assert_eq!(index.entity_for_output_name("HDMI-A-1"), None);
    }
}
