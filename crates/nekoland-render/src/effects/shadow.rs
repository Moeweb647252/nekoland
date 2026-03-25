//! Shadow effect feature registration and request emission.
#![allow(missing_docs)]

use bevy_app::App;
use bevy_ecs::prelude::{Res, ResMut, Resource};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::schedules::RenderSchedule;
use nekoland_ecs::resources::{
    RenderBindGroupLayoutKey, RenderMaterialKind, RenderMaterialParamBlock,
    RenderMaterialQueueKind, RenderMaterialShaderSource, RenderSceneRole, ShadowMaterialParams,
};

use crate::compositor_render::RenderViewSnapshot;
use crate::material::{
    RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequestQueue,
    RenderMaterialSpec, queue_typed_material_request,
};
use crate::plugin::RenderPrepareSystems;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowPostProcessMaterial;

impl RenderMaterialSpec for ShadowPostProcessMaterial {
    type Params = ShadowMaterialParams;

    const DEBUG_NAME: &'static str = "shadow";
    const MATERIAL_KIND: RenderMaterialKind = RenderMaterialKind::Shadow;
    const SHADER_SOURCE: RenderMaterialShaderSource = RenderMaterialShaderSource::Shadow;
    const BIND_GROUP_LAYOUT: RenderBindGroupLayoutKey = RenderBindGroupLayoutKey::ShadowUniforms;
    const QUEUE_KIND: RenderMaterialQueueKind = RenderMaterialQueueKind::PostProcess;

    fn to_param_block(params: Self::Params) -> RenderMaterialParamBlock {
        RenderMaterialParamBlock::Shadow(params)
    }
}

#[derive(Debug, Default, Clone, Copy)]
/// Feature plugin that wires shadow config and shadow request emission into the render pipeline.
pub struct ShadowEffectPlugin;

impl ShadowEffectPlugin {
    /// Installs the shared shadow configuration resource on the main world.
    pub fn init_config(app: &mut App) {
        app.init_resource::<ShadowEffectConfig>();
    }

    /// Installs shadow request emitters inside the render sub-app.
    pub fn install_render_subapp(app: &mut App) {
        app.init_resource::<ShadowEffectConfig>()
            .add_systems(RenderSchedule, shadow_effect_system.in_set(RenderPrepareSystems));
    }
}

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
    views: Res<'_, RenderViewSnapshot>,
    config: Res<'_, ShadowEffectConfig>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    if !config.enabled {
        return;
    }

    for output_id in views.views.iter().map(|view| view.output_id) {
        queue_typed_material_request::<ShadowPostProcessMaterial>(
            output_id,
            RenderSceneRole::Desktop,
            Some(ShadowMaterialParams {
                spread: config.spread,
                offset: [config.offset_x, config.offset_y],
                color: config.color,
            }),
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

    use super::{ShadowEffectConfig, shadow_effect_system};

    #[test]
    fn shadow_effect_queues_generic_material_request() {
        let mut world = World::default();
        world.insert_resource(RenderViewSnapshot {
            views: vec![RenderViewState {
                output_id: OutputId(9),
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1,
            }],
        });
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
            Some(&RenderMaterialParamBlock::shadow(3.0, 1.0, 2.0, [0.0, 0.0, 0.0, 0.25]))
        );
    }
}
