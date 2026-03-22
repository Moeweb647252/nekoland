use bevy_app::App;
use bevy_ecs::prelude::{Res, ResMut, Resource};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::schedules::RenderSchedule;
use nekoland_ecs::resources::{
    BlurMaterialParams, ProcessRect, RenderBindGroupLayoutKey, RenderMaterialKind,
    RenderMaterialParamBlock, RenderMaterialQueueKind, RenderMaterialShaderSource, RenderPlan,
    RenderPlanItem, RenderSceneRole,
};

use crate::compositor_render::RenderViewSnapshot;
use crate::material::{
    RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequestQueue,
    RenderMaterialSpec, queue_typed_material_request,
};
use crate::plugin::RenderPrepareSystems;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackdropBlurMaterial;

impl RenderMaterialSpec for BackdropBlurMaterial {
    type Params = BlurMaterialParams;

    const DEBUG_NAME: &'static str = "backdrop_blur";
    const MATERIAL_KIND: RenderMaterialKind = RenderMaterialKind::BackdropBlur;
    const SHADER_SOURCE: RenderMaterialShaderSource = RenderMaterialShaderSource::BackdropBlur;
    const BIND_GROUP_LAYOUT: RenderBindGroupLayoutKey = RenderBindGroupLayoutKey::BlurUniforms;
    const QUEUE_KIND: RenderMaterialQueueKind = RenderMaterialQueueKind::BackdropPostProcess;

    fn to_param_block(params: Self::Params) -> RenderMaterialParamBlock {
        RenderMaterialParamBlock::Blur(params)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlurPostProcessMaterial;

impl RenderMaterialSpec for BlurPostProcessMaterial {
    type Params = BlurMaterialParams;

    const DEBUG_NAME: &'static str = "blur";
    const MATERIAL_KIND: RenderMaterialKind = RenderMaterialKind::Blur;
    const SHADER_SOURCE: RenderMaterialShaderSource = RenderMaterialShaderSource::Blur;
    const BIND_GROUP_LAYOUT: RenderBindGroupLayoutKey = RenderBindGroupLayoutKey::BlurUniforms;
    const QUEUE_KIND: RenderMaterialQueueKind = RenderMaterialQueueKind::PostProcess;

    fn to_param_block(params: Self::Params) -> RenderMaterialParamBlock {
        RenderMaterialParamBlock::Blur(params)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BlurEffectPlugin;

impl BlurEffectPlugin {
    pub fn init_config(app: &mut App) {
        app.init_resource::<BlurEffectConfig>();
    }

    pub fn install_render_subapp(app: &mut App) {
        app.init_resource::<BlurEffectConfig>()
            .add_systems(RenderSchedule, blur_effect_system.in_set(RenderPrepareSystems))
            .add_systems(
                RenderSchedule,
                backdrop_blur_effect_system
                    .after(crate::compositor_render::assemble_render_plan_from_snapshot_system)
                    .in_set(RenderPrepareSystems),
            );
    }
}

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
    views: Res<'_, RenderViewSnapshot>,
    config: Res<'_, BlurEffectConfig>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    if !config.enabled {
        return;
    }

    for output_id in views.views.iter().map(|view| view.output_id) {
        queue_typed_material_request::<BlurPostProcessMaterial>(
            output_id,
            RenderSceneRole::Desktop,
            Some(BlurMaterialParams { radius: config.radius }),
            Vec::new(),
            &mut registry,
            &mut params_store,
            &mut requests,
        );
    }
}

/// Emits one controlled backdrop-blur request for every output that currently carries a backdrop
/// scene item.
pub fn backdrop_blur_effect_system(
    render_plan: Res<'_, RenderPlan>,
    mut registry: ResMut<'_, RenderMaterialRegistry>,
    mut params_store: ResMut<'_, RenderMaterialParamsStore>,
    mut requests: ResMut<'_, RenderMaterialRequestQueue>,
) {
    for (output_id, output_plan) in &render_plan.outputs {
        let has_backdrop = output_plan
            .ordered_item_ids()
            .iter()
            .filter_map(|item_id| output_plan.item(*item_id))
            .any(|item| matches!(item, RenderPlanItem::Backdrop(_)));
        if !has_backdrop {
            continue;
        }
        let process_regions = output_plan
            .iter_ordered()
            .filter_map(|item| match item {
                RenderPlanItem::Backdrop(item) => {
                    item.instance.visible_rect().map(ProcessRect::from)
                }
                RenderPlanItem::Surface(_)
                | RenderPlanItem::SolidRect(_)
                | RenderPlanItem::Cursor(_) => None,
            })
            .collect::<Vec<_>>();

        queue_typed_material_request::<BackdropBlurMaterial>(
            *output_id,
            RenderSceneRole::Compositor,
            Some(BlurMaterialParams { radius: 12.0 }),
            process_regions,
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
    use nekoland_ecs::resources::{
        BackdropRenderItem, OutputRenderPlan, RenderItemId, RenderItemIdentity, RenderItemInstance,
        RenderMaterialParamBlock, RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole,
        RenderSourceId,
    };

    use crate::compositor_render::{RenderViewSnapshot, RenderViewState};
    use crate::material::{
        RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequestQueue,
    };

    use super::{BlurEffectConfig, backdrop_blur_effect_system, blur_effect_system};

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[test]
    fn blur_effect_queues_generic_material_request() {
        let mut world = World::default();
        world.insert_resource(RenderViewSnapshot {
            views: vec![RenderViewState {
                output_id: OutputId(7),
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
                scale: 1,
            }],
        });
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
            Some(&RenderMaterialParamBlock::blur(6.0))
        );
    }

    #[test]
    fn backdrop_items_emit_backdrop_blur_requests() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(9),
                OutputRenderPlan::from_items([RenderPlanItem::Backdrop(BackdropRenderItem {
                    identity: identity(1),
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 200, height: 120 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Compositor,
                    },
                })]),
            )]),
        });
        world.init_resource::<RenderMaterialRegistry>();
        world.init_resource::<RenderMaterialParamsStore>();
        world.init_resource::<RenderMaterialRequestQueue>();

        let mut system = IntoSystem::into_system(backdrop_blur_effect_system);
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
            Some("backdrop_blur")
        );
        assert_eq!(
            request.params_id.and_then(|params_id| params_store.get(params_id)),
            Some(&RenderMaterialParamBlock::blur(12.0))
        );
    }
}
