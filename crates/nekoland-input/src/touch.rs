use bevy_ecs::prelude::Res;
use nekoland_ecs::resources::CompositorClock;

pub fn touch_input_system(clock: Res<CompositorClock>) {
    tracing::trace!(frame = clock.frame, "touch input system tick");
}
