use std::collections::BTreeSet;

use bevy_ecs::prelude::{Query, Res, ResMut, With};
use nekoland_ecs::components::{WlSurfaceHandle, XdgPopup, XdgWindow};
use nekoland_ecs::resources::{DamageState, FramePacingState, RenderList};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameCallbackDispatcher;

pub fn frame_callback_system(
    render_list: Res<RenderList>,
    surfaces: Query<&WlSurfaceHandle, With<XdgWindow>>,
    popups: Query<&WlSurfaceHandle, With<XdgPopup>>,
    mut damage_state: ResMut<DamageState>,
    mut frame_pacing: ResMut<FramePacingState>,
) {
    let callback_surface_ids = render_list
        .elements
        .iter()
        .filter_map(|element| (element.surface_id != 0).then_some(element.surface_id))
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
    damage_state.full_redraw = false;
}
