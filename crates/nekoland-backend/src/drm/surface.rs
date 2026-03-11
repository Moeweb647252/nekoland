use bevy_ecs::prelude::{Local, Query, Res};
use nekoland_ecs::components::{OutputDevice, OutputProperties};
use nekoland_ecs::resources::PendingOutputPresentationEvents;
use smithay::utils::{Clock, Monotonic};

use crate::plugin::{OutputPresentationRuntime, emit_present_completion_events};
use crate::traits::{BackendKind, SelectedBackend};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrmSurface {
    pub connector: String,
}

pub fn drm_surface_system() {
    tracing::trace!("drm surface system tick");
}

pub(crate) fn drm_present_completion_system(
    selected_backend: Res<SelectedBackend>,
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    mut pending_presentation_events: bevy_ecs::prelude::ResMut<PendingOutputPresentationEvents>,
    mut presentation_runtime: Local<OutputPresentationRuntime>,
    mut monotonic_clock: Local<Option<Clock<Monotonic>>>,
) {
    emit_present_completion_events(
        BackendKind::Drm,
        &selected_backend,
        &outputs,
        &mut pending_presentation_events,
        &mut presentation_runtime,
        &mut monotonic_clock,
    );
}
