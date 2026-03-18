use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::RenderSceneRole;

use crate::material::{
    RenderMaterialParams, RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequest,
    RenderMaterialRequestQueue,
};

/// Blur effect configuration.
///
/// # FUTURE: Implementation guide
///
/// When implementing GPU-side blur:
/// 1. In `blur_effect_system`, read `Res<BlurEffectConfig>` to check `enabled`.
/// 2. Register or reuse a render-local material id through `RenderMaterialRegistry`.
/// 3. Store the current frame's blur parameters in `RenderMaterialParamsStore`.
/// 4. Emit output-local material requests that the graph builder can project into generic
///    post-process passes.
///
/// `radius` controls the blur kernel size in pixels.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct BlurEffect {
    pub radius: f32,
}

/// Global config for the blur effect (read by the system when implemented).
#[allow(dead_code)]
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct BlurEffectConfig {
    pub enabled: bool,
    pub radius: f32,
}

impl Default for BlurEffectConfig {
    fn default() -> Self {
        Self { enabled: false, radius: 8.0 }
    }
}

/// Collects render-local blur material requests for the generic graph builder.
pub fn blur_effect_system(
    outputs: Query<'_, '_, &'static OutputId>,
    config: Res<'_, BlurEffectConfig>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    if !config.enabled {
        return;
    }

    let material_id = registry.register_named("blur");
    for output_id in outputs.iter().copied() {
        let params_id = params_store.insert(RenderMaterialParams::Blur { radius: config.radius });
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

    use super::{BlurEffectConfig, blur_effect_system};

    #[test]
    fn blur_effect_queues_generic_material_request() {
        let mut world = World::default();
        world.spawn(OutputId(7));
        world.insert_resource(BlurEffectConfig { enabled: true, radius: 6.0 });
        world.init_resource::<RenderMaterialRegistry>();
        world.init_resource::<RenderMaterialParamsStore>();
        world.init_resource::<RenderMaterialRequestQueue>();

        let mut system = IntoSystem::into_system(blur_effect_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let queue = world.resource::<RenderMaterialRequestQueue>();
        let request = &queue.outputs[&OutputId(7)][0];
        let registry = world.resource::<RenderMaterialRegistry>();
        let params_store = world.resource::<RenderMaterialParamsStore>();

        assert_eq!(
            registry
                .definition(request.material_id)
                .map(|definition| definition.debug_name.as_str()),
            Some("blur")
        );
        assert_eq!(
            request.params_id.and_then(|params_id| params_store.get(params_id)),
            Some(&RenderMaterialParams::Blur { radius: 6.0 })
        );
    }
}
