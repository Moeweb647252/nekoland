use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    BackdropRenderItem, CompositorSceneItem, CompositorSceneState, CursorRenderItem,
    CursorRenderSource, QuadContent, QuadRenderItem, RenderItemId, RenderItemIdentity,
    RenderItemInstance, RenderPlanItem, RenderSourceId, SurfacePresentationRole, SurfaceRenderItem,
};

use crate::scene_process::{
    AppearanceSnapshot, ProjectionSnapshot, apply_appearance_snapshot, apply_projection_snapshot,
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

    pub fn window(surface_id: u64) -> Self {
        Self::new("window", surface_id.to_string())
    }

    pub fn popup(surface_id: u64) -> Self {
        Self::new("popup", surface_id.to_string())
    }

    pub fn layer(surface_id: u64) -> Self {
        Self::new("layer", surface_id.to_string())
    }

    pub fn output_background(surface_id: u64) -> Self {
        Self::new("output_background", surface_id.to_string())
    }

    pub fn cursor_primary() -> Self {
        Self::new("cursor", "primary")
    }

    pub fn surface_for_role(surface_id: u64, role: SurfacePresentationRole) -> Self {
        match role {
            SurfacePresentationRole::Window => Self::window(surface_id),
            SurfacePresentationRole::Popup => Self::popup(surface_id),
            SurfacePresentationRole::Layer => Self::layer(surface_id),
            SurfacePresentationRole::OutputBackground => Self::output_background(surface_id),
        }
    }

    pub fn compositor(entry_id: nekoland_ecs::resources::CompositorSceneEntryId) -> Self {
        Self::new("compositor", entry_id.0.to_string())
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

    pub fn compositor(
        entry_id: nekoland_ecs::resources::CompositorSceneEntryId,
        output_id: OutputId,
    ) -> Self {
        Self::new(RenderSourceKey::compositor(entry_id), output_id, 0)
    }
}

/// Provider-local payload emitted into the scene contribution queue before final plan assembly.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderSceneContributionPayload {
    Surface { surface_id: u64 },
    Quad { content: QuadContent },
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
        source_key: RenderSourceKey,
        surface_id: u64,
        instance_slot: u32,
        instance: RenderItemInstance,
    ) -> Self {
        Self {
            key: RenderInstanceKey::new(source_key, output_id, instance_slot),
            instance,
            payload: RenderSceneContributionPayload::Surface { surface_id },
        }
    }

    pub fn quad(
        key: RenderInstanceKey,
        content: QuadContent,
        instance: RenderItemInstance,
    ) -> Self {
        Self { key, instance, payload: RenderSceneContributionPayload::Quad { content } }
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
            key: RenderInstanceKey::new(RenderSourceKey::cursor_primary(), output_id, 0),
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

/// Emits compositor-owned ECS scene state into the frame-local queue after desktop providers run.
pub fn emit_compositor_scene_contributions_system(
    compositor_scene: Option<bevy_ecs::prelude::Res<'_, CompositorSceneState>>,
    appearance: Option<Res<'_, AppearanceSnapshot>>,
    projection: Option<Res<'_, ProjectionSnapshot>>,
    mut queue: ResMut<'_, RenderSceneContributionQueue>,
) {
    let Some(compositor_scene) = compositor_scene else {
        return;
    };
    let appearance = appearance.as_deref();
    let projection = projection.as_deref();

    for (output_id, output_scene) in &compositor_scene.outputs {
        let output_contributions = queue.outputs.entry(*output_id).or_default();
        for (entry_id, entry) in output_scene.iter_ordered() {
            debug_assert!(
                matches!(
                    entry.instance.scene_role,
                    nekoland_ecs::resources::RenderSceneRole::Compositor
                        | nekoland_ecs::resources::RenderSceneRole::Overlay
                ),
                "compositor scene entries may only use compositor or overlay roles"
            );

            let key = RenderInstanceKey::compositor(entry_id, *output_id);
            let mut instance = entry.instance;
            apply_appearance_snapshot(&mut instance.opacity, &key.source_key, &key, appearance);
            apply_projection_snapshot(
                &mut instance.rect,
                &mut instance.clip_rect,
                &key.source_key,
                &key,
                projection,
            );

            let contribution = match &entry.item {
                CompositorSceneItem::Quad { content } => {
                    RenderSceneContribution::quad(key, content.clone(), instance)
                }
                CompositorSceneItem::Backdrop => RenderSceneContribution::backdrop(key, instance),
            };
            output_contributions.push(contribution);
        }
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
        RenderSceneContributionPayload::Quad { ref content } => {
            RenderPlanItem::Quad(QuadRenderItem {
                identity,
                content: content.clone(),
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
    use nekoland_ecs::resources::{
        QuadContent, RenderColor, RenderItemInstance, RenderRect, RenderSceneRole,
    };

    use super::{
        RenderInstanceKey, RenderSceneContribution, RenderSceneContributionPayload,
        RenderSceneContributionQueue, RenderSceneIdentityRegistry, RenderSourceKey,
        clear_scene_contributions_system, contribution_to_plan_item,
        emit_compositor_scene_contributions_system,
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
    fn role_specific_surface_source_keys_are_distinct() {
        assert_ne!(RenderSourceKey::window(7), RenderSourceKey::popup(7));
        assert_ne!(RenderSourceKey::window(7), RenderSourceKey::layer(7));
        assert_ne!(RenderSourceKey::window(7), RenderSourceKey::output_background(7));
        assert_ne!(RenderSourceKey::cursor_primary(), RenderSourceKey::window(7));
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
            payload: RenderSceneContributionPayload::Quad {
                content: QuadContent::SolidColor { color: RenderColor { r: 1, g: 2, b: 3, a: 4 } },
            },
        };

        let item = contribution_to_plan_item(&contribution, &mut registry);
        let nekoland_ecs::resources::RenderPlanItem::Quad(item) = item else {
            panic!("expected quad item");
        };
        assert_eq!(item.identity.source_id.0, 1);
        assert_eq!(item.identity.item_id.0, 1);
        assert_eq!(
            item.content,
            QuadContent::SolidColor { color: RenderColor { r: 1, g: 2, b: 3, a: 4 } }
        );
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

    #[test]
    fn compositor_scene_entries_emit_into_frame_local_contribution_queue() {
        let mut world = World::default();
        world.insert_resource(nekoland_ecs::resources::CompositorSceneState {
            outputs: BTreeMap::from([(
                OutputId(5),
                nekoland_ecs::resources::OutputCompositorScene::from_entries([(
                    nekoland_ecs::resources::CompositorSceneEntryId(9),
                    nekoland_ecs::resources::CompositorSceneEntry::solid_color(
                        RenderColor { r: 1, g: 2, b: 3, a: 4 },
                        RenderItemInstance {
                            rect: RenderRect { x: 6, y: 7, width: 8, height: 9 },
                            opacity: 0.75,
                            clip_rect: None,
                            z_index: 2,
                            scene_role: RenderSceneRole::Compositor,
                        },
                    ),
                )]),
            )]),
        });
        world.init_resource::<RenderSceneContributionQueue>();

        let mut system = IntoSystem::into_system(emit_compositor_scene_contributions_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let queue = world.resource::<RenderSceneContributionQueue>();
        let contributions = queue.outputs.get(&OutputId(5)).expect("output contributions");
        assert_eq!(contributions.len(), 1);
        assert_eq!(
            contributions[0].key.source_key,
            RenderSourceKey::compositor(nekoland_ecs::resources::CompositorSceneEntryId(9))
        );
        assert!(matches!(contributions[0].payload, RenderSceneContributionPayload::Quad { .. }));
    }
}
