use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    MaterialParamsId, ProcessRect, RenderBindGroupLayoutKey, RenderMaterialDescriptor,
    RenderMaterialFrameState, RenderMaterialId, RenderMaterialKind, RenderMaterialParamBlock,
    RenderMaterialPipelineKey, RenderMaterialQueueKind, RenderMaterialShaderSource,
    RenderSceneRole,
};

pub trait RenderMaterialSpec {
    type Params;

    const DEBUG_NAME: &'static str;
    const MATERIAL_KIND: RenderMaterialKind;
    const SHADER_SOURCE: RenderMaterialShaderSource;
    const BIND_GROUP_LAYOUT: RenderBindGroupLayoutKey;
    const QUEUE_KIND: RenderMaterialQueueKind;

    fn pipeline_key() -> RenderMaterialPipelineKey {
        RenderMaterialPipelineKey::post_process(Self::MATERIAL_KIND)
    }

    fn to_param_block(params: Self::Params) -> RenderMaterialParamBlock;

    fn queue_request(
        output_id: OutputId,
        scene_role: RenderSceneRole,
        params: Option<Self::Params>,
        registry: &mut RenderMaterialRegistry,
        params_store: &mut RenderMaterialParamsStore,
        requests: &mut RenderMaterialRequestQueue,
    ) -> RenderMaterialRequest
    where
        Self: Sized,
    {
        let material_id = registry.register_typed::<Self>().id();
        let params_id = params.map(|params| params_store.insert_typed::<Self>(params));
        let request = RenderMaterialRequest {
            scene_role,
            material_id,
            params_id,
            process_regions: Vec::new(),
        };
        requests.outputs.entry(output_id).or_default().push(request.clone());
        request
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredMaterial<M> {
    pub material_id: RenderMaterialId,
    _marker: PhantomData<M>,
}

impl<M> RegisteredMaterial<M> {
    pub fn id(self) -> RenderMaterialId {
        self.material_id
    }
}

/// One render-local material definition keyed by an opaque `RenderMaterialId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMaterialDefinition {
    pub debug_name: String,
    pub pipeline_key: RenderMaterialPipelineKey,
    pub shader_source: RenderMaterialShaderSource,
    pub bind_group_layout: RenderBindGroupLayoutKey,
    pub queue_kind: RenderMaterialQueueKind,
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
        self.register_named_with_pipeline_key(
            debug_name,
            RenderMaterialPipelineKey::post_process(RenderMaterialKind::Generic),
        )
    }

    pub fn register_named_with_pipeline_key(
        &mut self,
        debug_name: &str,
        pipeline_key: RenderMaterialPipelineKey,
    ) -> RenderMaterialId {
        self.register_definition(
            debug_name,
            RenderMaterialDefinition {
                debug_name: debug_name.to_owned(),
                pipeline_key,
                shader_source: RenderMaterialShaderSource::Generic,
                bind_group_layout: RenderBindGroupLayoutKey::Generic,
                queue_kind: RenderMaterialQueueKind::PostProcess,
            },
        )
    }

    pub fn register_definition(
        &mut self,
        debug_name: &str,
        definition: RenderMaterialDefinition,
    ) -> RenderMaterialId {
        if let Some(id) = self.ids_by_name.get(debug_name).copied() {
            return id;
        }

        let id = RenderMaterialId(self.next_id.max(1));
        self.next_id = id.0.saturating_add(1);
        self.ids_by_name.insert(debug_name.to_owned(), id);
        self.definitions.insert(id, definition);
        id
    }

    pub fn register_typed<M>(&mut self) -> RegisteredMaterial<M>
    where
        M: RenderMaterialSpec,
    {
        RegisteredMaterial {
            material_id: self.register_definition(
                M::DEBUG_NAME,
                RenderMaterialDefinition {
                    debug_name: M::DEBUG_NAME.to_owned(),
                    pipeline_key: M::pipeline_key(),
                    shader_source: M::SHADER_SOURCE,
                    bind_group_layout: M::BIND_GROUP_LAYOUT,
                    queue_kind: M::QUEUE_KIND,
                },
            ),
            _marker: PhantomData,
        }
    }

    pub fn definition(&self, id: RenderMaterialId) -> Option<&RenderMaterialDefinition> {
        self.definitions.get(&id)
    }
}

/// Frame-local parameter store keyed by opaque ids.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct RenderMaterialParamsStore {
    next_id: u64,
    params: BTreeMap<MaterialParamsId, RenderMaterialParamBlock>,
}

impl RenderMaterialParamsStore {
    pub fn insert(&mut self, params: RenderMaterialParamBlock) -> MaterialParamsId {
        let id = MaterialParamsId(self.next_id.max(1));
        self.next_id = id.0.saturating_add(1);
        self.params.insert(id, params);
        id
    }

    pub fn get(&self, id: MaterialParamsId) -> Option<&RenderMaterialParamBlock> {
        self.params.get(&id)
    }

    pub fn insert_typed<M>(&mut self, params: M::Params) -> MaterialParamsId
    where
        M: RenderMaterialSpec,
    {
        self.insert(M::to_param_block(params))
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
    pub process_regions: Vec<ProcessRect>,
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

pub fn queue_typed_material_request<M>(
    output_id: OutputId,
    scene_role: RenderSceneRole,
    params: Option<M::Params>,
    process_regions: Vec<ProcessRect>,
    registry: &mut RenderMaterialRegistry,
    params_store: &mut RenderMaterialParamsStore,
    requests: &mut RenderMaterialRequestQueue,
) -> RenderMaterialRequest
where
    M: RenderMaterialSpec,
{
    let material_id = registry.register_typed::<M>().id();
    let params_id = params.map(|params| params_store.insert_typed::<M>(params));
    let request = RenderMaterialRequest { scene_role, material_id, params_id, process_regions };
    requests.outputs.entry(output_id).or_default().push(request.clone());
    request
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
                        shader_source: definition.shader_source.clone(),
                        bind_group_layout: definition.bind_group_layout.clone(),
                        queue_kind: definition.queue_kind.clone(),
                    },
                )
            })
        })
        .collect();
    materials.params = referenced_params
        .into_iter()
        .filter_map(|params_id| {
            params_store.get(params_id).cloned().map(|block| (params_id, block))
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::System;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        BlurMaterialParams, MaterialParamsId, RenderBindGroupLayoutKey, RenderMaterialFrameState,
        RenderMaterialId, RenderMaterialKind, RenderMaterialParamBlock, RenderMaterialQueueKind,
        RenderMaterialShaderSource, RenderPipelineStage, RenderSceneRole,
    };

    use crate::effects::blur::{BackdropBlurMaterial, BlurPostProcessMaterial};

    use super::{
        RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequest,
        RenderMaterialRequestQueue, project_material_frame_state_system,
        queue_typed_material_request,
    };

    #[test]
    fn registry_reuses_ids_for_same_name() {
        let mut registry = RenderMaterialRegistry::default();

        let blur_a = registry.register_typed::<BlurPostProcessMaterial>().id();
        let blur_b = registry.register_typed::<BlurPostProcessMaterial>().id();
        let shadow = registry.register_named("shadow");

        assert_eq!(blur_a, blur_b);
        assert_ne!(blur_a, shadow);
        assert_eq!(
            registry.definition(blur_a).map(|definition| definition.pipeline_key.material),
            Some(RenderMaterialKind::Blur)
        );
        assert_eq!(
            registry.definition(blur_a).map(|definition| (
                definition.shader_source.clone(),
                definition.bind_group_layout.clone(),
                definition.queue_kind.clone()
            )),
            Some((
                RenderMaterialShaderSource::Blur,
                RenderBindGroupLayoutKey::BlurUniforms,
                RenderMaterialQueueKind::PostProcess
            ))
        );
    }

    #[test]
    fn params_store_allocates_opaque_ids() {
        let mut store = RenderMaterialParamsStore::default();

        let first =
            store.insert_typed::<BlurPostProcessMaterial>(BlurMaterialParams { radius: 8.0 });
        let second = store.insert(RenderMaterialParamBlock::default());

        assert_eq!(first, MaterialParamsId(1));
        assert_eq!(second, MaterialParamsId(2));
        assert_eq!(store.get(first), Some(&RenderMaterialParamBlock::blur(8.0)));
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
                    process_regions: Vec::new(),
                }],
            )]),
        };

        assert_eq!(
            queue.outputs[&OutputId(7)][0],
            RenderMaterialRequest {
                scene_role: RenderSceneRole::Desktop,
                material_id: RenderMaterialId(1),
                params_id: Some(MaterialParamsId(2)),
                process_regions: Vec::new(),
            }
        );
    }

    #[test]
    fn typed_queue_helper_registers_material_and_params_in_one_step() {
        let mut registry = RenderMaterialRegistry::default();
        let mut params_store = RenderMaterialParamsStore::default();
        let mut queue = RenderMaterialRequestQueue::default();

        let request = queue_typed_material_request::<BackdropBlurMaterial>(
            OutputId(3),
            RenderSceneRole::Compositor,
            Some(BlurMaterialParams { radius: 9.0 }),
            Vec::new(),
            &mut registry,
            &mut params_store,
            &mut queue,
        );

        assert_eq!(
            registry
                .definition(request.material_id)
                .map(|definition| definition.shader_source.clone()),
            Some(RenderMaterialShaderSource::BackdropBlur)
        );
        assert_eq!(
            request.params_id.and_then(|params_id| params_store.get(params_id)),
            Some(&RenderMaterialParamBlock::blur(9.0))
        );
        assert_eq!(queue.outputs[&OutputId(3)][0], request);
    }

    #[test]
    fn material_frame_state_projects_generic_descriptors_and_params() {
        let mut world = World::default();
        let mut registry = RenderMaterialRegistry::default();
        let material_id = registry.register_typed::<BackdropBlurMaterial>().id();
        let mut params_store = RenderMaterialParamsStore::default();
        let params_id =
            params_store.insert_typed::<BackdropBlurMaterial>(BlurMaterialParams { radius: 14.0 });
        world.insert_resource(registry);
        world.insert_resource(params_store);
        world.insert_resource(RenderMaterialRequestQueue {
            outputs: BTreeMap::from([(
                OutputId(1),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Compositor,
                    material_id,
                    params_id: Some(params_id),
                    process_regions: Vec::new(),
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
            frame_state.descriptor(material_id).map(|descriptor| descriptor.pipeline_key.material),
            Some(RenderMaterialKind::BackdropBlur)
        );
        assert_eq!(
            frame_state.params(params_id).and_then(RenderMaterialParamBlock::radius),
            Some(14.0)
        );
    }

    #[test]
    fn typed_material_registration_uses_specialized_pipeline_key() {
        let mut registry = RenderMaterialRegistry::default();

        let material = registry.register_typed::<BackdropBlurMaterial>();

        assert_eq!(
            registry.definition(material.id()).map(|definition| {
                (
                    definition.pipeline_key.material,
                    definition.pipeline_key.stage,
                    definition.shader_source.clone(),
                    definition.bind_group_layout.clone(),
                    definition.queue_kind.clone(),
                )
            }),
            Some((
                RenderMaterialKind::BackdropBlur,
                RenderPipelineStage::PostProcess,
                RenderMaterialShaderSource::BackdropBlur,
                RenderBindGroupLayoutKey::BlurUniforms,
                RenderMaterialQueueKind::BackdropPostProcess,
            ))
        );
    }
}
