use bevy_ecs::prelude::Res;
use nekoland_ecs::resources::CompositorClock;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenshotService;

pub fn screenshot_system(clock: Res<CompositorClock>) {
    tracing::trace!(frame = clock.frame, "screenshot system tick");
}
