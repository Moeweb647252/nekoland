use bevy_ecs::prelude::Res;
use nekoland_ecs::resources::CompositorClock;

/// Placeholder touch stage.
///
/// Touch-specific state is not modeled yet, but the system keeps the input schedule shape stable
/// and provides a tracing hook for future touch integration.
pub fn touch_input_system(clock: Res<CompositorClock>) {
    tracing::trace!(frame = clock.frame, "touch input system tick");
}
