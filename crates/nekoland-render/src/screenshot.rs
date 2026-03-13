use bevy_ecs::prelude::Res;
use nekoland_ecs::resources::CompositorClock;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenshotService;

/// Placeholder screenshot stage.
///
/// The service is not implemented yet, but the system keeps the render schedule shape stable and
/// provides a trace point for future screenshot capture work.
pub fn screenshot_system(clock: Res<CompositorClock>) {
    tracing::trace!(frame = clock.frame, "screenshot system tick");
}
