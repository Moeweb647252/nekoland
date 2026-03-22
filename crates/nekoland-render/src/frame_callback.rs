use std::collections::BTreeSet;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    DamageState, FramePacingState, RenderPlan, RenderPlanItem, ShellRenderInput,
    SurfacePresentationRole,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameCallbackDispatcher;

/// Computes which surfaces should receive frame callbacks from the current render plan and marks
/// the rest as throttled for pacing diagnostics.
pub fn frame_callback_system(
    render_plan: Res<RenderPlan>,
    shell_render_input: Option<Res<'_, ShellRenderInput>>,
    mut damage_state: ResMut<DamageState>,
    mut frame_pacing: ResMut<FramePacingState>,
) {
    let callback_surface_ids = render_plan
        .outputs
        .values()
        .flat_map(|plan| plan.iter_ordered())
        .filter_map(|item| match item {
            RenderPlanItem::Surface(item) => Some(item.surface_id),
            RenderPlanItem::SolidRect(_)
            | RenderPlanItem::Backdrop(_)
            | RenderPlanItem::Cursor(_) => None,
        })
        .collect::<BTreeSet<_>>();
    let surface_presentation =
        shell_render_input.as_deref().map(|mailbox| &mailbox.surface_presentation);
    let known_surface_ids = surface_presentation
        .map(|snapshot| {
            snapshot
                .surfaces
                .iter()
                .filter_map(|(surface_id, state)| {
                    matches!(
                        state.role,
                        SurfacePresentationRole::Window | SurfacePresentationRole::Popup
                    )
                    .then_some(*surface_id)
                })
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    frame_pacing.callback_surface_ids = callback_surface_ids.iter().copied().collect();
    frame_pacing.throttled_surface_ids =
        known_surface_ids.difference(&callback_surface_ids).copied().collect();
    if !frame_pacing.callback_surface_ids.is_empty() {
        frame_pacing.frame_callbacks_sent = frame_pacing
            .frame_callbacks_sent
            .saturating_add(frame_pacing.callback_surface_ids.len() as u64);
    }

    tracing::trace!(
        callbacks = frame_pacing.callback_surface_ids.len(),
        throttled = frame_pacing.throttled_surface_ids.len(),
        "frame callback tick"
    );
    // Once callback recipients are derived for this frame, the next damage decision should be
    // based on fresh scene changes rather than the previous frame's redraw marker.
    damage_state.full_redraw = false;
}
