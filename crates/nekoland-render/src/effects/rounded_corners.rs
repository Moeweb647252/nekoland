use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::RenderSceneRole;

use crate::material::{
    RenderMaterialParams, RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequest,
    RenderMaterialRequestQueue,
};

/// Rounded corner effect configuration.
///
/// # FUTURE: Implementation guide
///
/// When implementing rounded corners:
/// 1. In `rounded_corner_effect_system`, read `Res<RoundedCornerEffectConfig>`.
/// 2. Register or reuse a render-local material id through `RenderMaterialRegistry`.
/// 3. Store the current frame's rounded-corner parameters in `RenderMaterialParamsStore`.
/// 4. Emit output-local material requests that the graph builder can project into generic
///    post-process passes.
///
/// `radius` is the corner radius in logical pixels.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundedCornerEffect {
    pub radius: f32,
}

/// Global config for rounded corners.
#[allow(dead_code)]
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct RoundedCornerEffectConfig {
    pub enabled: bool,
    pub radius: f32,
}

impl Default for RoundedCornerEffectConfig {
    fn default() -> Self {
        Self { enabled: false, radius: 8.0 }
    }
}

/// Collects render-local rounded-corner material requests for the generic graph builder.
pub fn rounded_corner_effect_system(
    outputs: Query<'_, '_, &'static OutputId>,
    config: Res<'_, RoundedCornerEffectConfig>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    if !config.enabled {
        return;
    }

    let material_id = registry.register_named("rounded_corners");
    for output_id in outputs.iter().copied() {
        let params_id =
            params_store.insert(RenderMaterialParams::RoundedCorners { radius: config.radius });
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

    use super::{RoundedCornerEffectConfig, rounded_corner_effect_system};

    #[test]
    fn rounded_corner_effect_queues_generic_material_request() {
        let mut world = World::default();
        world.spawn(OutputId(5));
        world.insert_resource(RoundedCornerEffectConfig { enabled: true, radius: 12.0 });
        world.init_resource::<RenderMaterialRegistry>();
        world.init_resource::<RenderMaterialParamsStore>();
        world.init_resource::<RenderMaterialRequestQueue>();

        let mut system = IntoSystem::into_system(rounded_corner_effect_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let queue = world.resource::<RenderMaterialRequestQueue>();
        let request = &queue.outputs[&OutputId(5)][0];
        let registry = world.resource::<RenderMaterialRegistry>();
        let params_store = world.resource::<RenderMaterialParamsStore>();

        assert_eq!(
            registry
                .definition(request.material_id)
                .map(|definition| definition.debug_name.as_str()),
            Some("rounded_corners")
        );
        assert_eq!(
            request.params_id.and_then(|params_id| params_store.get(params_id)),
            Some(&RenderMaterialParams::RoundedCorners { radius: 12.0 })
        );
    }
}
