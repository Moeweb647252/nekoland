use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{PreRenderSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    CompletedScreenshotFrames, CompositorSceneState, CursorImageSnapshot, CursorSceneSnapshot,
    DamageState, FramePacingState, OutputDamageRegions, PendingScreenshotRequests,
    RenderMaterialFrameState, RenderPassGraph, RenderPlan, SurfaceVisualSnapshot,
};

use crate::{
    animation, compositor_render, cursor, damage_tracker, effects, frame_callback, material,
    presentation_feedback, render_graph, scene_source, screenshot, surface_visual,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPlugin;

impl NekolandPlugin for RenderPlugin {
    /// Register render-stage resources plus the strictly ordered render pipeline
    /// that keeps compositor-internal rendering separate from user-facing visual state.
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<material::RenderMaterialRegistry>()
            .init_resource::<material::RenderMaterialParamsStore>()
            .init_resource::<material::RenderMaterialRequestQueue>()
            .init_resource::<RenderMaterialFrameState>()
            .init_resource::<PendingScreenshotRequests>()
            .init_resource::<CompletedScreenshotFrames>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<scene_source::RenderSceneContributionQueue>()
            .init_resource::<scene_source::RenderSceneIdentityRegistry>()
            .init_resource::<animation::AnimationTimelineStore>()
            .init_resource::<CursorSceneSnapshot>()
            .init_resource::<CursorImageSnapshot>()
            .init_resource::<cursor::CursorThemeGeometryCache>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<SurfaceVisualSnapshot>()
            .init_resource::<effects::blur::BlurEffectConfig>()
            .init_resource::<effects::shadow::ShadowEffectConfig>()
            .init_resource::<effects::rounded_corners::RoundedCornerEffectConfig>()
            .add_systems(
                PreRenderSchedule,
                (
                    effects::fade::fade_effect_system,
                    animation::advance_animation_timelines_system,
                    surface_visual::surface_visual_snapshot_system,
                    material::clear_material_requests_system,
                    effects::blur::blur_effect_system,
                    effects::shadow::shadow_effect_system,
                    effects::rounded_corners::rounded_corner_effect_system,
                )
                    .chain(),
            )
            .add_systems(
                RenderSchedule,
                // Core rendering stays linear on purpose: damage/render-list/cursor/pacing all
                // build on the state produced by the previous internal stage.
                (
                    scene_source::clear_scene_contributions_system,
                    compositor_render::emit_desktop_scene_contributions_system,
                    scene_source::emit_compositor_scene_contributions_system,
                    cursor::cursor_scene_snapshot_system,
                    cursor::emit_cursor_scene_contributions_system,
                    compositor_render::assemble_render_plan_system,
                    material::emit_backdrop_material_requests_system,
                    material::project_material_frame_state_system,
                    render_graph::build_render_graph_system,
                    damage_tracker::damage_tracking_system,
                    frame_callback::frame_callback_system,
                    presentation_feedback::presentation_feedback_system,
                    screenshot::screenshot_system,
                )
                    .chain(),
            );
    }
}
