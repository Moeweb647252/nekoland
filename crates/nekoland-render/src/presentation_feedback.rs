use bevy_ecs::prelude::ResMut;
use nekoland_ecs::resources::FramePacingState;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresentationFeedbackDispatcher;

pub fn presentation_feedback_system(mut frame_pacing: ResMut<FramePacingState>) {
    frame_pacing.presentation_surface_ids = frame_pacing.callback_surface_ids.clone();
    if !frame_pacing.presentation_surface_ids.is_empty() {
        frame_pacing.presentation_batches = frame_pacing.presentation_batches.saturating_add(1);
    }

    tracing::trace!(
        presented = frame_pacing.presentation_surface_ids.len(),
        throttled = frame_pacing.throttled_surface_ids.len(),
        "presentation feedback tick"
    );
}
