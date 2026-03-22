use bevy_ecs::prelude::Res;
use nekoland_ecs::resources::{CompositorClock, ShellRenderInput};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenshotService;

/// Screenshot/readback service stage.
///
/// The actual pixel extraction happens inside backend present once the render graph reaches a
/// readback pass. This system keeps request/result resources visible in the render schedule and
/// provides a trace point for debugging the internal service.
pub fn screenshot_system(clock: Res<CompositorClock>, shell_render_input: Res<ShellRenderInput>) {
    tracing::trace!(
        frame = clock.frame,
        pending_requests = shell_render_input.pending_screenshot_requests.requests.len(),
        "screenshot system tick"
    );
}
