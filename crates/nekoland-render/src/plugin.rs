use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{PreRenderSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    CursorRenderState, DamageState, FramePacingState, OutputDamageRegions, RenderPassGraph,
    RenderPlan, RenderPlanInjectionState, SurfaceVisualSnapshot,
};

use crate::{
    compositor_render, cursor, damage_tracker, effects, frame_callback, material,
    presentation_feedback, render_graph, screenshot, surface_visual,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPlugin;

impl NekolandPlugin for RenderPlugin {
    /// Register render-stage resources plus the strictly ordered render pipeline
    /// that keeps compositor-internal rendering separate from user-facing visual state.
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPlanInjectionState>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<material::RenderMaterialRegistry>()
            .init_resource::<material::RenderMaterialParamsStore>()
            .init_resource::<material::RenderMaterialRequestQueue>()
            .init_resource::<CursorRenderState>()
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
                    compositor_render::compose_frame_system,
                    render_graph::build_render_graph_system,
                    damage_tracker::damage_tracking_system,
                    cursor::cursor_render_system,
                    frame_callback::frame_callback_system,
                    presentation_feedback::presentation_feedback_system,
                    screenshot::screenshot_system,
                )
                    .chain(),
            );
    }
}
