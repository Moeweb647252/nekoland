use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{PostRenderSchedule, PreRenderSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    CursorRenderState, DamageState, FramePacingState, OutputDamageRegions, RenderPlan,
    SurfaceVisualSnapshot,
};

use crate::{
    compositor_render, cursor, damage_tracker, effects, frame_callback, presentation_feedback,
    screenshot, surface_visual,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPlugin;

impl NekolandPlugin for RenderPlugin {
    /// Register render-stage resources plus the strictly ordered render pipeline
    /// that keeps compositor-internal rendering separate from user-facing visual state.
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<CursorRenderState>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<SurfaceVisualSnapshot>()
            .add_systems(
                PreRenderSchedule,
                (effects::fade::fade_effect_system, surface_visual::surface_visual_snapshot_system)
                    .chain(),
            )
            .add_systems(
                RenderSchedule,
                // Core rendering stays linear on purpose: damage/render-list/cursor/pacing all
                // build on the state produced by the previous internal stage.
                (
                    damage_tracker::damage_tracking_system,
                    compositor_render::compose_frame_system,
                    cursor::cursor_render_system,
                    frame_callback::frame_callback_system,
                    presentation_feedback::presentation_feedback_system,
                    screenshot::screenshot_system,
                )
                    .chain(),
            )
            .add_systems(
                PostRenderSchedule,
                (
                    effects::blur::blur_effect_system,
                    effects::shadow::shadow_effect_system,
                    effects::rounded_corners::rounded_corner_effect_system,
                )
                    .chain(),
            );
    }
}
