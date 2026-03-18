use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::RenderSceneRole;

use crate::material::{
    RenderMaterialParams, RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequest,
    RenderMaterialRequestQueue,
};

/// Shadow effect configuration.
///
/// # FUTURE: Implementation guide
///
/// When implementing drop shadows:
/// 1. In `shadow_effect_system`, read `Res<ShadowEffectConfig>`.
/// 2. Register or reuse a render-local material id through `RenderMaterialRegistry`.
/// 3. Store the current frame's shadow parameters in `RenderMaterialParamsStore`.
/// 4. Emit output-local material requests that the graph builder can project into generic
///    post-process passes.
///
/// `spread` is the shadow blur radius in pixels; `offset` is the drop offset.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowEffect {
    pub spread: f32,
}

/// Global config for the shadow effect.
#[allow(dead_code)]
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ShadowEffectConfig {
    pub enabled: bool,
    pub spread: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub color: [f32; 4],
}

impl Default for ShadowEffectConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            spread: 12.0,
            offset_x: 0.0,
            offset_y: 4.0,
            color: [0.0, 0.0, 0.0, 0.5],
        }
    }
}

/// Collects render-local shadow material requests for the generic graph builder.
pub fn shadow_effect_system(
    outputs: Query<'_, '_, &'static OutputId>,
    config: Res<'_, ShadowEffectConfig>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    if !config.enabled {
        return;
    }

    let material_id = registry.register_named("shadow");
    for output_id in outputs.iter().copied() {
        let params_id = params_store.insert(RenderMaterialParams::Shadow {
            spread: config.spread,
            offset_x: config.offset_x,
            offset_y: config.offset_y,
            color: config.color,
        });
        requests.outputs.entry(output_id).or_default().push(RenderMaterialRequest {
            scene_role: RenderSceneRole::Desktop,
            material_id,
            params_id: Some(params_id),
        });
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;

    use crate::material::{
        RenderMaterialParams, RenderMaterialParamsStore, RenderMaterialRegistry,
        RenderMaterialRequestQueue,
    };

    use super::{ShadowEffectConfig, shadow_effect_system};

    #[test]
    fn shadow_effect_queues_generic_material_request() {
        let mut world = World::default();
        world.spawn(OutputId(9));
        world.insert_resource(ShadowEffectConfig {
            enabled: true,
            spread: 3.0,
            offset_x: 1.0,
            offset_y: 2.0,
            color: [0.0, 0.0, 0.0, 0.25],
        });
        world.init_resource::<RenderMaterialRegistry>();
        world.init_resource::<RenderMaterialParamsStore>();
        world.init_resource::<RenderMaterialRequestQueue>();

        let mut system = IntoSystem::into_system(shadow_effect_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let queue = world.resource::<RenderMaterialRequestQueue>();
        let request = &queue.outputs[&OutputId(9)][0];
        let registry = world.resource::<RenderMaterialRegistry>();
        let params_store = world.resource::<RenderMaterialParamsStore>();

        assert_eq!(
            registry
                .definition(request.material_id)
                .map(|definition| definition.debug_name.as_str()),
            Some("shadow")
        );
        assert_eq!(
            request.params_id.and_then(|params_id| params_store.get(params_id)),
            Some(&RenderMaterialParams::Shadow {
                spread: 3.0,
                offset_x: 1.0,
                offset_y: 2.0,
                color: [0.0, 0.0, 0.0, 0.25],
            })
        );
    }
}
