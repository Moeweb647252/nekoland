use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::RenderSchedule;
use nekoland_ecs::resources::{DamageState, FramePacingState, OutputDamageRegions, RenderList};

use crate::{
    compositor_render, cursor, damage_tracker, effects, frame_callback, presentation_feedback,
    screenshot,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPlugin;

impl NekolandPlugin for RenderPlugin {
    /// Register render-stage resources plus the strictly ordered render pipeline
    /// that derives damage, render lists, callbacks, and post-processing.
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderList>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .add_systems(
                RenderSchedule,
                // Rendering stays linear on purpose: damage/render-list/cursor/frame-callback and
                // post-processing steps all build on the state produced by the previous stage.
                (
                    damage_tracker::damage_tracking_system,
                    compositor_render::compose_frame_system,
                    cursor::cursor_render_system,
                    frame_callback::frame_callback_system,
                    presentation_feedback::presentation_feedback_system,
                    screenshot::screenshot_system,
                    effects::blur::blur_effect_system,
                    effects::shadow::shadow_effect_system,
                    effects::rounded_corners::rounded_corner_effect_system,
                    effects::fade::fade_effect_system,
                )
                    .chain(),
            );
    }
}
