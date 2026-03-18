use std::collections::BTreeMap;

use bevy_ecs::prelude::{ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    BackdropRenderItem, CursorRenderItem, CursorRenderSource, RenderColor, RenderItemId,
    RenderItemIdentity, RenderItemInstance, RenderPlanItem, RenderSourceId, SolidRectRenderItem,
    SurfaceRenderItem,
};

/// Stable render-local source key resolved into ECS-facing `RenderSourceId`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenderSourceKey {
    pub namespace: String,
    pub local_key: String,
}

impl RenderSourceKey {
    pub fn new(namespace: impl Into<String>, local_key: impl Into<String>) -> Self {
        Self { namespace: namespace.into(), local_key: local_key.into() }
    }

    pub fn surface(surface_id: u64) -> Self {
        Self::new("surface", surface_id.to_string())
    }
}

/// Stable render-local output-local instance key resolved into ECS-facing `RenderItemId`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenderInstanceKey {
    pub source_key: RenderSourceKey,
    pub output_id: OutputId,
    pub instance_slot: u32,
}

impl RenderInstanceKey {
    pub fn new(source_key: RenderSourceKey, output_id: OutputId, instance_slot: u32) -> Self {
        Self { source_key, output_id, instance_slot }
    }
}

/// Provider-local payload emitted into the scene contribution queue before final plan assembly.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderSceneContributionPayload {
    Surface { surface_id: u64 },
    SolidRect { color: RenderColor },
    Backdrop,
    Cursor { source: CursorRenderSource },
}

/// One output-local scene contribution awaiting stable-id resolution and render-plan assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSceneContribution {
    pub key: RenderInstanceKey,
    pub instance: RenderItemInstance,
    pub payload: RenderSceneContributionPayload,
}

impl RenderSceneContribution {
    pub fn surface(
        output_id: OutputId,
        surface_id: u64,
        instance_slot: u32,
        instance: RenderItemInstance,
    ) -> Self {
        Self {
            key: RenderInstanceKey::new(
                RenderSourceKey::surface(surface_id),
                output_id,
                instance_slot,
            ),
            instance,
            payload: RenderSceneContributionPayload::Surface { surface_id },
        }
    }

    pub fn solid_rect(
        key: RenderInstanceKey,
        color: RenderColor,
        instance: RenderItemInstance,
    ) -> Self {
        Self { key, instance, payload: RenderSceneContributionPayload::SolidRect { color } }
    }

    pub fn backdrop(key: RenderInstanceKey, instance: RenderItemInstance) -> Self {
        Self { key, instance, payload: RenderSceneContributionPayload::Backdrop }
    }

    pub fn cursor(
        output_id: OutputId,
        source: CursorRenderSource,
        instance: RenderItemInstance,
    ) -> Self {
        Self {
            key: RenderInstanceKey::new(RenderSourceKey::new("cursor", "primary"), output_id, 0),
            instance,
            payload: RenderSceneContributionPayload::Cursor { source },
        }
    }
}

/// Frame-local output-scoped scene contributions emitted by render-side producers.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct RenderSceneContributionQueue {
    pub outputs: BTreeMap<OutputId, Vec<RenderSceneContribution>>,
}

/// Persistent external scene contributions merged into the frame-local queue each render tick.
///
/// This provides a stable provider boundary for tests and future compositor-owned producers
/// without letting them bypass scene assembly and stable-id resolution.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct ExternalSceneContributionState {
    pub outputs: BTreeMap<OutputId, Vec<RenderSceneContribution>>,
}

/// Persistent render-local registry that resolves stable provider keys into ECS-facing ids.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RenderSceneIdentityRegistry {
    next_source_id: u64,
    next_item_id: u64,
    source_ids: BTreeMap<RenderSourceKey, RenderSourceId>,
    item_ids: BTreeMap<RenderInstanceKey, RenderItemId>,
}

impl RenderSceneIdentityRegistry {
    pub fn source_id_for(&mut self, key: &RenderSourceKey) -> RenderSourceId {
        if let Some(id) = self.source_ids.get(key).copied() {
            return id;
        }

        let id = RenderSourceId(self.next_source_id.max(1));
        self.next_source_id = id.0.saturating_add(1);
        self.source_ids.insert(key.clone(), id);
        id
    }

    pub fn item_id_for(&mut self, key: &RenderInstanceKey) -> RenderItemId {
        if let Some(id) = self.item_ids.get(key).copied() {
            return id;
        }

        let id = RenderItemId(self.next_item_id.max(1));
        self.next_item_id = id.0.saturating_add(1);
        self.item_ids.insert(key.clone(), id);
        id
    }

    pub fn identity_for(&mut self, key: &RenderInstanceKey) -> RenderItemIdentity {
        RenderItemIdentity {
            source_id: self.source_id_for(&key.source_key),
            item_id: self.item_id_for(key),
        }
    }
}

/// Clears frame-local scene contributions before scene providers rebuild them.
pub fn clear_scene_contributions_system(mut queue: ResMut<'_, RenderSceneContributionQueue>) {
    queue.outputs.clear();
}

/// Emits persistent external contributions into the frame-local queue after providers are reset.
pub fn emit_external_scene_contributions_system(
    external: Option<bevy_ecs::prelude::Res<'_, ExternalSceneContributionState>>,
    mut queue: ResMut<'_, RenderSceneContributionQueue>,
) {
    let Some(external) = external else {
        return;
    };

    for (output_id, contributions) in &external.outputs {
        queue.outputs.entry(*output_id).or_default().extend(contributions.iter().cloned());
    }
}

pub fn contribution_to_plan_item(
    contribution: &RenderSceneContribution,
    identity_registry: &mut RenderSceneIdentityRegistry,
) -> RenderPlanItem {
    let identity = identity_registry.identity_for(&contribution.key);
    match contribution.payload {
        RenderSceneContributionPayload::Surface { surface_id } => {
            RenderPlanItem::Surface(SurfaceRenderItem {
                identity,
                surface_id,
                instance: contribution.instance,
            })
        }
        RenderSceneContributionPayload::SolidRect { color } => {
            RenderPlanItem::SolidRect(SolidRectRenderItem {
                identity,
                color,
                instance: contribution.instance,
            })
        }
        RenderSceneContributionPayload::Backdrop => RenderPlanItem::Backdrop(BackdropRenderItem {
            identity,
            instance: contribution.instance,
        }),
        RenderSceneContributionPayload::Cursor { ref source } => {
            RenderPlanItem::Cursor(CursorRenderItem {
                identity,
                source: source.clone(),
                instance: contribution.instance,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{RenderColor, RenderItemInstance, RenderRect, RenderSceneRole};

    use super::{
        RenderInstanceKey, RenderSceneContribution, RenderSceneContributionPayload,
        RenderSceneContributionQueue, RenderSceneIdentityRegistry, RenderSourceKey,
        clear_scene_contributions_system, contribution_to_plan_item,
    };

    #[test]
    fn identity_registry_reuses_ids_for_same_provider_keys() {
        let mut registry = RenderSceneIdentityRegistry::default();
        let key = RenderInstanceKey::new(RenderSourceKey::surface(42), OutputId(7), 0);

        let first = registry.identity_for(&key);
        let second = registry.identity_for(&key);

        assert_eq!(first, second);
    }

    #[test]
    fn instance_key_changes_item_id_but_not_source_id() {
        let mut registry = RenderSceneIdentityRegistry::default();
        let source_key = RenderSourceKey::surface(42);
        let first =
            registry.identity_for(&RenderInstanceKey::new(source_key.clone(), OutputId(7), 0));
        let second =
            registry.identity_for(&RenderInstanceKey::new(source_key.clone(), OutputId(7), 1));
        let third = registry.identity_for(&RenderInstanceKey::new(source_key, OutputId(8), 0));

        assert_eq!(first.source_id, second.source_id);
        assert_eq!(first.source_id, third.source_id);
        assert_ne!(first.item_id, second.item_id);
        assert_ne!(first.item_id, third.item_id);
    }

    #[test]
    fn contribution_to_plan_item_preserves_identity_and_payload() {
        let mut registry = RenderSceneIdentityRegistry::default();
        let contribution = RenderSceneContribution {
            key: RenderInstanceKey::new(RenderSourceKey::new("test", "solid"), OutputId(1), 0),
            instance: RenderItemInstance {
                rect: RenderRect { x: 1, y: 2, width: 3, height: 4 },
                opacity: 0.5,
                clip_rect: None,
                z_index: 9,
                scene_role: RenderSceneRole::Overlay,
            },
            payload: RenderSceneContributionPayload::SolidRect {
                color: RenderColor { r: 1, g: 2, b: 3, a: 4 },
            },
        };

        let item = contribution_to_plan_item(&contribution, &mut registry);
        let nekoland_ecs::resources::RenderPlanItem::SolidRect(item) = item else {
            panic!("expected solid rect item");
        };
        assert_eq!(item.identity.source_id.0, 1);
        assert_eq!(item.identity.item_id.0, 1);
        assert_eq!(item.color, RenderColor { r: 1, g: 2, b: 3, a: 4 });
        assert_eq!(item.instance.opacity, 0.5);
    }

    #[test]
    fn clear_scene_contributions_empties_all_outputs() {
        let mut world = World::default();
        world.insert_resource(RenderSceneContributionQueue {
            outputs: BTreeMap::from([(
                OutputId(1),
                vec![RenderSceneContribution::backdrop(
                    RenderInstanceKey::new(
                        RenderSourceKey::new("test", "backdrop"),
                        OutputId(1),
                        0,
                    ),
                    RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 1, height: 1 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Overlay,
                    },
                )],
            )]),
        });

        let mut system = IntoSystem::into_system(clear_scene_contributions_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        assert!(world.resource::<RenderSceneContributionQueue>().outputs.is_empty());
    }
}
