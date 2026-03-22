use bevy_app::App;
use bevy_ecs::prelude::{Res, ResMut, Resource};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::schedules::RenderSchedule;
use nekoland_ecs::resources::{
    RenderBindGroupLayoutKey, RenderMaterialKind, RenderMaterialParamBlock,
    RenderMaterialQueueKind, RenderMaterialShaderSource, RenderSceneRole,
    RoundedCornerMaterialParams,
};

use crate::compositor_render::RenderViewSnapshot;
use crate::material::{
    RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequestQueue,
    RenderMaterialSpec, queue_typed_material_request,
};
use crate::plugin::RenderPrepareSystems;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundedCornerMaskMaterial;

impl RenderMaterialSpec for RoundedCornerMaskMaterial {
    type Params = RoundedCornerMaterialParams;

    const DEBUG_NAME: &'static str = "rounded_corners";
    const MATERIAL_KIND: RenderMaterialKind = RenderMaterialKind::RoundedCorners;
    const SHADER_SOURCE: RenderMaterialShaderSource = RenderMaterialShaderSource::RoundedCorners;
    const BIND_GROUP_LAYOUT: RenderBindGroupLayoutKey =
        RenderBindGroupLayoutKey::RoundedCornerUniforms;
    const QUEUE_KIND: RenderMaterialQueueKind = RenderMaterialQueueKind::Mask;

    fn to_param_block(params: Self::Params) -> RenderMaterialParamBlock {
        RenderMaterialParamBlock::RoundedCorners(params)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RoundedCornerEffectPlugin;

impl RoundedCornerEffectPlugin {
    pub fn init_config(app: &mut App) {
        app.init_resource::<RoundedCornerEffectConfig>();
    }

    pub fn install_render_subapp(app: &mut App) {
        app.init_resource::<RoundedCornerEffectConfig>()
            .add_systems(RenderSchedule, rounded_corner_effect_system.in_set(RenderPrepareSystems));
    }
}

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
    views: Res<'_, RenderViewSnapshot>,
    config: Res<'_, RoundedCornerEffectConfig>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    if !config.enabled {
        return;
    }

    for output_id in views.views.iter().map(|view| view.output_id) {
        queue_typed_material_request::<RoundedCornerMaskMaterial>(
            output_id,
            RenderSceneRole::Desktop,
            Some(RoundedCornerMaterialParams { radius: config.radius }),
            Vec::new(),
            &mut registry,
            &mut params_store,
            &mut requests,
        );
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::RenderMaterialParamBlock;

    use crate::compositor_render::{RenderViewSnapshot, RenderViewState};
    use crate::material::{
        RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequestQueue,
    };

    use super::{RoundedCornerEffectConfig, rounded_corner_effect_system};

    #[test]
    fn rounded_corner_effect_queues_generic_material_request() {
        let mut world = World::default();
        world.insert_resource(RenderViewSnapshot {
            views: vec![RenderViewState {
                output_id: OutputId(5),
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1,
            }],
        });
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
            Some(&RenderMaterialParamBlock::rounded_corners(12.0))
        );
    }
}
