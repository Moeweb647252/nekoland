use std::collections::BTreeSet;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Query, Res, ResMut, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{WlSurfaceHandle, XdgPopup, XdgWindow};
use nekoland_ecs::resources::{DamageState, FramePacingState, RenderPlan, RenderPlanItem};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameCallbackDispatcher;

/// Computes which surfaces should receive frame callbacks from the current render plan and marks
/// the rest as throttled for pacing diagnostics.
pub fn frame_callback_system(
    render_plan: Res<RenderPlan>,
    surfaces: Query<&WlSurfaceHandle, (With<XdgWindow>, Allow<Disabled>)>,
    popups: Query<&WlSurfaceHandle, (With<XdgPopup>, Allow<Disabled>)>,
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
    let known_surface_ids =
        surfaces.iter().chain(popups.iter()).map(|surface| surface.id).collect::<BTreeSet<_>>();

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
