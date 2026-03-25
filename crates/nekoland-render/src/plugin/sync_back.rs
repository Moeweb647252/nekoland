//! Sync-back helpers that mirror render-world products into the main world.

use bevy_ecs::prelude::{Res, ResMut};
use bevy_ecs::schedule::InternedScheduleLabel;
use bevy_ecs::world::World;
use nekoland_ecs::resources::{
    CompiledOutputFrames, DamageState, FramePacingState, OutputDamageRegions, PreparedGpuResources,
    PreparedSceneResources, RenderFinalOutputPlan, RenderMaterialFrameState, RenderPassGraph,
    RenderProcessPlan, RenderReadbackPlan, RenderTargetAllocationPlan, SurfaceTextureBridgePlan,
};

/// Mirrors render-world resources back into the main world after the render sub-app runs.
pub(super) fn sync_render_subapp_back(
    main_world: &mut World,
    render_world: &mut World,
    _schedule: Option<InternedScheduleLabel>,
) {
    super::clone_resource_into::<RenderMaterialFrameState>(render_world, main_world);
    super::clone_resource_into::<RenderPassGraph>(render_world, main_world);
    super::clone_resource_into::<RenderProcessPlan>(render_world, main_world);
    super::clone_resource_into::<RenderFinalOutputPlan>(render_world, main_world);
    super::clone_resource_into::<RenderReadbackPlan>(render_world, main_world);
    super::clone_resource_into::<CompiledOutputFrames>(render_world, main_world);
    super::clone_resource_into::<DamageState>(render_world, main_world);
    super::clone_resource_into::<FramePacingState>(render_world, main_world);
    super::clone_resource_into::<OutputDamageRegions>(render_world, main_world);
    super::clone_resource_into::<crate::pipeline_cache::RenderPipelineCacheState>(
        render_world,
        main_world,
    );
    super::clone_resource_into::<PreparedSceneResources>(render_world, main_world);
    super::clone_resource_into::<PreparedGpuResources>(render_world, main_world);
    super::clone_resource_into::<RenderTargetAllocationPlan>(render_world, main_world);
    super::clone_resource_into::<SurfaceTextureBridgePlan>(render_world, main_world);
}

/// Rebuilds the aggregated `CompiledOutputFrames` resource from render-world sub-results.
pub(super) fn sync_compiled_output_frames_system(
    output_damage_regions: Res<'_, OutputDamageRegions>,
    prepared_scene: Res<'_, PreparedSceneResources>,
    prepared_gpu: Res<'_, PreparedGpuResources>,
    materials: Res<'_, RenderMaterialFrameState>,
    render_graph: Res<'_, RenderPassGraph>,
    render_plan: Res<'_, nekoland_ecs::resources::RenderPlan>,
    process_plan: Res<'_, RenderProcessPlan>,
    final_output_plan: Res<'_, RenderFinalOutputPlan>,
    readback_plan: Res<'_, RenderReadbackPlan>,
    render_target_allocation: Res<'_, RenderTargetAllocationPlan>,
    surface_texture_bridge: Res<'_, SurfaceTextureBridgePlan>,
    mut compiled: ResMut<'_, CompiledOutputFrames>,
) {
    let outputs = render_plan
        .outputs
        .iter()
        .map(|(output_id, output_render_plan)| {
            (
                *output_id,
                nekoland_ecs::resources::CompiledOutputFrame {
                    render_plan: output_render_plan.clone(),
                    prepared_scene: prepared_scene
                        .outputs
                        .get(output_id)
                        .cloned()
                        .unwrap_or_default(),
                    execution_plan: render_graph
                        .outputs
                        .get(output_id)
                        .cloned()
                        .unwrap_or_default(),
                    process_plan: process_plan.outputs.get(output_id).cloned().unwrap_or_default(),
                    final_output: final_output_plan.outputs.get(output_id).cloned(),
                    readback: readback_plan.outputs.get(output_id).cloned(),
                    target_allocation: render_target_allocation.outputs.get(output_id).cloned(),
                    gpu_prep: prepared_gpu.outputs.get(output_id).cloned(),
                    damage_regions: output_damage_regions
                        .regions
                        .get(output_id)
                        .cloned()
                        .unwrap_or_default(),
                },
            )
        })
        .collect();

    *compiled = CompiledOutputFrames {
        outputs,
        output_damage_regions: output_damage_regions.clone(),
        prepared_scene: prepared_scene.clone(),
        materials: materials.clone(),
        render_graph: render_graph.clone(),
        render_plan: render_plan.clone(),
        process_plan: process_plan.clone(),
        final_output_plan: final_output_plan.clone(),
        readback_plan: readback_plan.clone(),
        render_target_allocation: render_target_allocation.clone(),
        surface_texture_bridge: surface_texture_bridge.clone(),
        prepared_gpu: prepared_gpu.clone(),
    };
}
