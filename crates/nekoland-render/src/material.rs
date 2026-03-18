use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    MaterialParamsId, RenderMaterialDescriptor, RenderMaterialFrameState, RenderMaterialId,
    RenderMaterialParamBlock, RenderMaterialParamValue, RenderMaterialPipelineKey, RenderPlan,
    RenderPlanItem, RenderSceneRole,
};

/// Render-local parameter payloads keyed by opaque `MaterialParamsId` in the execution graph.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderMaterialParams {
    BackdropBlur { radius: f32 },
    Blur { radius: f32 },
    Shadow { spread: f32, offset_x: f32, offset_y: f32, color: [f32; 4] },
    RoundedCorners { radius: f32 },
    Passthrough,
}

/// One render-local material definition keyed by an opaque `RenderMaterialId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMaterialDefinition {
    pub debug_name: String,
    pub pipeline_key: RenderMaterialPipelineKey,
}

/// Persistent registry from human-readable render-local material names to opaque ids.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RenderMaterialRegistry {
    next_id: u64,
    ids_by_name: BTreeMap<String, RenderMaterialId>,
    definitions: BTreeMap<RenderMaterialId, RenderMaterialDefinition>,
}

impl RenderMaterialRegistry {
    pub fn register_named(&mut self, debug_name: &str) -> RenderMaterialId {
        if let Some(id) = self.ids_by_name.get(debug_name).copied() {
            return id;
        }

        let id = RenderMaterialId(self.next_id.max(1));
        self.next_id = id.0.saturating_add(1);
        self.ids_by_name.insert(debug_name.to_owned(), id);
        self.definitions.insert(
            id,
            RenderMaterialDefinition {
                debug_name: debug_name.to_owned(),
                pipeline_key: RenderMaterialPipelineKey(debug_name.to_owned()),
            },
        );
        id
    }

    pub fn definition(&self, id: RenderMaterialId) -> Option<&RenderMaterialDefinition> {
        self.definitions.get(&id)
    }
}

/// Frame-local parameter store keyed by opaque ids.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct RenderMaterialParamsStore {
    next_id: u64,
    params: BTreeMap<MaterialParamsId, RenderMaterialParams>,
}

impl RenderMaterialParamsStore {
    pub fn insert(&mut self, params: RenderMaterialParams) -> MaterialParamsId {
        let id = MaterialParamsId(self.next_id.max(1));
        self.next_id = id.0.saturating_add(1);
        self.params.insert(id, params);
        id
    }

    pub fn get(&self, id: MaterialParamsId) -> Option<&RenderMaterialParams> {
        self.params.get(&id)
    }

    pub fn clear_frame(&mut self) {
        self.next_id = 1;
        self.params.clear();
    }
}

/// One output-local material request waiting to be projected into the execution graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMaterialRequest {
    pub scene_role: RenderSceneRole,
    pub material_id: RenderMaterialId,
    pub params_id: Option<MaterialParamsId>,
}

/// Frame-local queue of output-scoped material requests emitted by render-side effect adapters.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct RenderMaterialRequestQueue {
    pub outputs: BTreeMap<OutputId, Vec<RenderMaterialRequest>>,
}

/// Clears per-frame material requests and parameter payloads before effect adapters rebuild them.
pub fn clear_material_requests_system(
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
    mut params: ResMut<'_, RenderMaterialParamsStore>,
) {
    requests.outputs.clear();
    params.clear_frame();
}

/// Emits one controlled backdrop-blur request for every output that currently carries a backdrop
/// scene item.
pub fn emit_backdrop_material_requests_system(
    render_plan: Res<'_, RenderPlan>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    let material_id = registry.register_named("backdrop_blur");

    for (output_id, output_plan) in &render_plan.outputs {
        let has_backdrop = output_plan
            .ordered_item_ids()
            .iter()
            .filter_map(|item_id| output_plan.item(*item_id))
            .any(|item| matches!(item, RenderPlanItem::Backdrop(_)));
        if !has_backdrop {
            continue;
        }

        let params_id = params_store.insert(RenderMaterialParams::BackdropBlur { radius: 12.0 });
        requests.outputs.entry(*output_id).or_default().push(RenderMaterialRequest {
            scene_role: RenderSceneRole::Compositor,
            material_id,
            params_id: Some(params_id),
        });
    }
}

/// Projects render-local material requests into backend-readable ECS frame state.
pub fn project_material_frame_state_system(
    registry: Res<'_, RenderMaterialRegistry>,
    params_store: Res<'_, RenderMaterialParamsStore>,
    requests: Res<'_, RenderMaterialRequestQueue>,
    mut materials: ResMut<'_, RenderMaterialFrameState>,
) {
    let referenced_materials = requests
        .outputs
        .values()
        .flat_map(|requests| requests.iter().map(|request| request.material_id))
        .collect::<BTreeSet<_>>();
    let referenced_params = requests
        .outputs
        .values()
        .flat_map(|requests| requests.iter().filter_map(|request| request.params_id))
        .collect::<BTreeSet<_>>();

    materials.descriptors = referenced_materials
        .into_iter()
        .filter_map(|material_id| {
            registry.definition(material_id).map(|definition| {
                (
                    material_id,
                    RenderMaterialDescriptor {
                        debug_name: definition.debug_name.clone(),
                        pipeline_key: definition.pipeline_key.clone(),
                    },
                )
            })
        })
        .collect();
    materials.params = referenced_params
        .into_iter()
        .filter_map(|params_id| {
            params_store
                .get(params_id)
                .map(render_material_param_block)
                .map(|block| (params_id, block))
        })
        .collect();
}

fn render_material_param_block(params: &RenderMaterialParams) -> RenderMaterialParamBlock {
    let values = match params {
        RenderMaterialParams::BackdropBlur { radius } | RenderMaterialParams::Blur { radius } => {
            BTreeMap::from([("radius".to_owned(), RenderMaterialParamValue::Float(*radius))])
        }
        RenderMaterialParams::Shadow { spread, offset_x, offset_y, color } => BTreeMap::from([
            ("spread".to_owned(), RenderMaterialParamValue::Float(*spread)),
            ("offset".to_owned(), RenderMaterialParamValue::Vec2([*offset_x, *offset_y])),
            ("color".to_owned(), RenderMaterialParamValue::Vec4(*color)),
        ]),
        RenderMaterialParams::RoundedCorners { radius } => {
            BTreeMap::from([("radius".to_owned(), RenderMaterialParamValue::Float(*radius))])
        }
        RenderMaterialParams::Passthrough => BTreeMap::default(),
    };

    RenderMaterialParamBlock { values }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::System;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        BackdropRenderItem, MaterialParamsId, OutputRenderPlan, RenderItemIdentity,
        RenderItemInstance, RenderMaterialFrameState, RenderMaterialId, RenderPlan, RenderPlanItem,
        RenderRect, RenderSceneRole, RenderSourceId,
    };

    use super::{
        RenderMaterialParams, RenderMaterialParamsStore, RenderMaterialRegistry,
        RenderMaterialRequest, RenderMaterialRequestQueue, emit_backdrop_material_requests_system,
        project_material_frame_state_system,
    };

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), nekoland_ecs::resources::RenderItemId(id))
    }

    #[test]
    fn registry_reuses_ids_for_same_name() {
        let mut registry = RenderMaterialRegistry::default();

        let blur_a = registry.register_named("blur");
        let blur_b = registry.register_named("blur");
        let shadow = registry.register_named("shadow");

        assert_eq!(blur_a, blur_b);
        assert_ne!(blur_a, shadow);
        assert_eq!(
            registry.definition(blur_a).map(|definition| definition.pipeline_key.0.as_str()),
            Some("blur")
        );
    }

    #[test]
    fn params_store_allocates_opaque_ids() {
        let mut store = RenderMaterialParamsStore::default();

        let first = store.insert(RenderMaterialParams::Blur { radius: 8.0 });
        let second = store.insert(RenderMaterialParams::Passthrough);

        assert_eq!(first, MaterialParamsId(1));
        assert_eq!(second, MaterialParamsId(2));
        assert_eq!(store.get(first), Some(&RenderMaterialParams::Blur { radius: 8.0 }));
    }

    #[test]
    fn request_queue_groups_by_output() {
        let queue = RenderMaterialRequestQueue {
            outputs: BTreeMap::from([(
                OutputId(7),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Desktop,
                    material_id: RenderMaterialId(1),
                    params_id: Some(MaterialParamsId(2)),
                }],
            )]),
        };

        assert_eq!(
            queue.outputs[&OutputId(7)][0],
            RenderMaterialRequest {
                scene_role: RenderSceneRole::Desktop,
                material_id: RenderMaterialId(1),
                params_id: Some(MaterialParamsId(2)),
            }
        );
    }

    #[test]
    fn backdrop_items_emit_backdrop_blur_requests() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputRenderPlan::from_items([RenderPlanItem::Backdrop(BackdropRenderItem {
                    identity: identity(11),
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 10,
                        scene_role: RenderSceneRole::Overlay,
                    },
                })]),
            )]),
        });
        world.init_resource::<RenderMaterialRegistry>();
        world.init_resource::<RenderMaterialParamsStore>();
        world.init_resource::<RenderMaterialRequestQueue>();

        let mut system =
            bevy_ecs::system::IntoSystem::into_system(emit_backdrop_material_requests_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let queue = world.resource::<RenderMaterialRequestQueue>();
        let registry = world.resource::<RenderMaterialRegistry>();
        let request = &queue.outputs[&OutputId(7)][0];
        assert_eq!(
            registry
                .definition(request.material_id)
                .map(|definition| definition.debug_name.as_str()),
            Some("backdrop_blur")
        );
    }

    #[test]
    fn material_frame_state_projects_generic_descriptors_and_params() {
        let mut world = World::default();
        let mut registry = RenderMaterialRegistry::default();
        let material_id = registry.register_named("backdrop_blur");
        let mut params_store = RenderMaterialParamsStore::default();
        let params_id = params_store.insert(RenderMaterialParams::BackdropBlur { radius: 14.0 });
        world.insert_resource(registry);
        world.insert_resource(params_store);
        world.insert_resource(RenderMaterialRequestQueue {
            outputs: BTreeMap::from([(
                OutputId(1),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Compositor,
                    material_id,
                    params_id: Some(params_id),
                }],
            )]),
        });
        world.init_resource::<RenderMaterialFrameState>();

        let mut system =
            bevy_ecs::system::IntoSystem::into_system(project_material_frame_state_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let frame_state = world.resource::<RenderMaterialFrameState>();
        assert_eq!(
            frame_state
                .descriptor(material_id)
                .map(|descriptor| descriptor.pipeline_key.0.as_str()),
            Some("backdrop_blur")
        );
        assert_eq!(
            frame_state.params(params_id).and_then(|block| block.float("radius")),
            Some(14.0)
        );
    }
}
