use std::collections::BTreeMap;

use bevy_ecs::prelude::{ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{MaterialParamsId, RenderMaterialId, RenderSceneRole};

/// Render-local parameter payloads keyed by opaque `MaterialParamsId` in the execution graph.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderMaterialParams {
    Blur { radius: f32 },
    Shadow { spread: f32, offset_x: f32, offset_y: f32, color: [f32; 4] },
    RoundedCorners { radius: f32 },
    Passthrough,
}

/// One render-local material definition keyed by an opaque `RenderMaterialId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMaterialDefinition {
    pub debug_name: String,
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
        self.definitions.insert(id, RenderMaterialDefinition { debug_name: debug_name.to_owned() });
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{MaterialParamsId, RenderMaterialId, RenderSceneRole};

    use super::{
        RenderMaterialParams, RenderMaterialParamsStore, RenderMaterialRegistry,
        RenderMaterialRequest, RenderMaterialRequestQueue,
    };

    #[test]
    fn registry_reuses_ids_for_same_name() {
        let mut registry = RenderMaterialRegistry::default();

        let blur_a = registry.register_named("blur");
        let blur_b = registry.register_named("blur");
        let shadow = registry.register_named("shadow");

        assert_eq!(blur_a, blur_b);
        assert_ne!(blur_a, shadow);
        assert_eq!(
            registry.definition(blur_a),
            Some(&super::RenderMaterialDefinition { debug_name: "blur".to_owned() })
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
}
