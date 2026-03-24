//! Render plugin wiring for both the main-world animation stage and the render sub-app.

use bevy_app::{App, SubApp};
use bevy_ecs::schedule::{InternedScheduleLabel, IntoScheduleConfigs, ScheduleLabel, SystemSet};
use bevy_ecs::world::World;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{PreRenderSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    CompiledOutputFrames, CompositorClock, CompositorSceneState, CursorSceneSnapshot, DamageState,
    FramePacingState, OutputDamageRegions, PreparedGpuResources, PreparedSceneResources,
    RenderFinalOutputPlan, RenderMaterialFrameState, RenderPassGraph, RenderPhasePlan, RenderPlan,
    RenderProcessPlan, RenderReadbackPlan, RenderTargetAllocationPlan, ShellRenderInput,
    SurfaceBufferAttachmentSnapshot, SurfaceContentVersionSnapshot, SurfaceTextureBridgePlan,
};

use crate::{
    animation, compositor_render, cursor, damage_tracker, effects, final_output_plan,
    frame_callback, material, output_overlay, overlay_ui, phase_plan, pipeline_cache,
    prepare_resources, presentation_feedback, process_plan, readback_plan, render_graph,
    scene_process, scene_source, screenshot,
};

pub mod extract;
pub mod sync_back;

#[derive(Debug, Default, Clone, Copy)]
/// Main-world plugin that owns shared render resources and pre-render animation updates.
pub struct RenderPlugin;

#[derive(Debug, Default, Clone, Copy)]
/// Render sub-app plugin that compiles shell snapshots into backend-neutral output frames.
pub struct RenderSubAppPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Render schedule set that extracts scene state and prepares per-item descriptors.
pub struct RenderPrepareSystems;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Render schedule set that compiles phases, graphs, and allocation plans.
pub struct RenderQueueSystems;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Render schedule set that computes damage and publishes compiled output frames.
pub struct RenderExecuteSystems;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Render schedule set that emits cleanup-stage feedback such as frame callbacks and screenshots.
pub struct RenderCleanupSystems;

impl NekolandPlugin for RenderPlugin {
    /// Register render-stage resources plus the strictly ordered render pipeline
    /// that keeps compositor-internal rendering separate from user-facing visual state.
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<RenderPhasePlan>()
            .init_resource::<RenderProcessPlan>()
            .init_resource::<material::RenderMaterialRegistry>()
            .init_resource::<material::RenderMaterialParamsStore>()
            .init_resource::<material::RenderMaterialRequestQueue>()
            .init_resource::<RenderMaterialFrameState>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<output_overlay::OutputOverlaySceneSyncState>()
            .init_resource::<overlay_ui::OverlayUiSceneSyncState>()
            .init_resource::<overlay_ui::OverlayTextRasterizerState>()
            .init_resource::<scene_source::RenderSceneContributionQueue>()
            .init_resource::<scene_source::RenderSceneIdentityRegistry>()
            .init_resource::<compositor_render::DesktopSurfaceOrderSnapshot>()
            .init_resource::<animation::AnimationTimelineStore>()
            .init_resource::<CursorSceneSnapshot>()
            .init_resource::<cursor::CursorThemeGeometryCache>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<scene_process::AppearanceSnapshot>()
            .init_resource::<scene_process::ProjectionSnapshot>()
            .init_resource::<pipeline_cache::RenderPipelineCacheState>()
            .init_resource::<PreparedSceneResources>()
            .init_resource::<PreparedGpuResources>()
            .init_resource::<RenderTargetAllocationPlan>()
            .init_resource::<SurfaceTextureBridgePlan>()
            .init_resource::<RenderFinalOutputPlan>()
            .init_resource::<RenderReadbackPlan>()
            .add_systems(
                PreRenderSchedule,
                (
                    animation::advance_animation_timelines_system,
                    scene_process::prune_stale_compositor_animation_tracks_system,
                )
                    .chain(),
            );

        effects::install_main_render_features(app);
    }
}

impl NekolandPlugin for RenderSubAppPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<RenderPhasePlan>()
            .init_resource::<RenderProcessPlan>()
            .init_resource::<material::RenderMaterialRegistry>()
            .init_resource::<material::RenderMaterialParamsStore>()
            .init_resource::<material::RenderMaterialRequestQueue>()
            .init_resource::<RenderMaterialFrameState>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<SurfaceContentVersionSnapshot>()
            .init_resource::<compositor_render::RenderViewSnapshot>()
            .init_resource::<compositor_render::DesktopSurfaceOrderSnapshot>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<CursorSceneSnapshot>()
            .init_resource::<CompositorClock>()
            .init_resource::<ShellRenderInput>()
            .init_resource::<output_overlay::OutputOverlaySceneSyncState>()
            .init_resource::<overlay_ui::OverlayUiSceneSyncState>()
            .init_resource::<overlay_ui::OverlayTextRasterizerState>()
            .init_resource::<scene_source::RenderSceneContributionQueue>()
            .init_resource::<scene_source::RenderSceneIdentityRegistry>()
            .init_resource::<scene_process::AppearanceSnapshot>()
            .init_resource::<scene_process::ProjectionSnapshot>()
            .init_resource::<cursor::CursorThemeGeometryCache>()
            .init_resource::<pipeline_cache::RenderPipelineCacheState>()
            .init_resource::<SurfaceBufferAttachmentSnapshot>()
            .init_resource::<PreparedSceneResources>()
            .init_resource::<PreparedGpuResources>()
            .init_resource::<RenderTargetAllocationPlan>()
            .init_resource::<SurfaceTextureBridgePlan>()
            .init_resource::<RenderFinalOutputPlan>()
            .init_resource::<RenderReadbackPlan>()
            .configure_sets(
                RenderSchedule,
                (
                    RenderPrepareSystems,
                    RenderQueueSystems.after(RenderPrepareSystems),
                    RenderExecuteSystems.after(RenderQueueSystems),
                    RenderCleanupSystems.after(RenderExecuteSystems),
                ),
            )
            .add_systems(
                RenderSchedule,
                (
                    material::clear_material_requests_system,
                    output_overlay::sync_output_overlay_scene_state_system,
                    overlay_ui::sync_overlay_ui_scene_state_system,
                    scene_source::clear_scene_contributions_system,
                    compositor_render::emit_desktop_scene_contributions_from_snapshot_system,
                    scene_source::emit_compositor_scene_contributions_system,
                    cursor::cursor_scene_snapshot_system,
                    cursor::emit_cursor_scene_contributions_system,
                    compositor_render::assemble_render_plan_from_snapshot_system,
                    prepare_resources::build_surface_texture_bridge_plan_system,
                    prepare_resources::build_prepared_scene_resources_system,
                    material::project_material_frame_state_system,
                )
                    .chain()
                    .in_set(RenderPrepareSystems),
            )
            .add_systems(
                RenderSchedule,
                (
                    phase_plan::build_render_phase_plan_system,
                    render_graph::build_render_graph_system,
                    final_output_plan::build_render_final_output_plan_system,
                    readback_plan::build_render_readback_plan_system,
                    process_plan::build_render_process_plan_system,
                    prepare_resources::build_render_target_allocation_plan_system,
                    prepare_resources::build_prepared_gpu_resources_system,
                    pipeline_cache::build_render_pipeline_cache_state_system,
                    pipeline_cache::build_process_pipeline_cache_state_system,
                )
                    .chain()
                    .in_set(RenderQueueSystems),
            )
            .add_systems(
                RenderSchedule,
                (
                    damage_tracker::damage_tracking_system,
                    sync_back::sync_compiled_output_frames_system,
                )
                    .chain()
                    .in_set(RenderExecuteSystems),
            )
            .add_systems(
                RenderSchedule,
                (
                    frame_callback::frame_callback_system,
                    presentation_feedback::presentation_feedback_system,
                    screenshot::screenshot_system,
                )
                    .chain()
                    .in_set(RenderCleanupSystems),
            );

        effects::install_render_subapp_features(app);
    }
}

/// Configures the render sub-app to run `RenderSchedule` and extract from the main world.
pub fn configure_render_subapp(sub_app: &mut SubApp) {
    sub_app.update_schedule = Some(RenderSchedule.intern());
    sub_app.set_extract(extract::extract_render_subapp_inputs);
}

/// Mirrors render-world products such as compiled frames and damage state back into the main world.
pub fn sync_render_subapp_back(
    main_world: &mut World,
    render_world: &mut World,
    schedule: Option<InternedScheduleLabel>,
) {
    sync_back::sync_render_subapp_back(main_world, render_world, schedule);
}

pub(crate) fn clone_resource_into<R>(source: &World, dest: &mut World)
where
    R: bevy_ecs::prelude::Resource + Clone,
{
    if let Some(resource) = source.get_resource::<R>() {
        dest.insert_resource(resource.clone());
    }
}

#[cfg(test)]
mod tests;
