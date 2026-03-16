/// Fade-in/fade-out animation driver.
///
/// # FUTURE: Implementation guide
///
/// When implementing fade animations:
/// 1. In `fade_effect_system`, query all windows for `&mut WindowAnimation`.
/// 2. For windows with `FadeState::In`, increment `elapsed_ms` by the frame
///    delta time (exposed via `Res<CompositorClock>`).
/// 3. Compute `progress = (elapsed_ms / duration_ms).min(1.0)`.
/// 4. When `elapsed_ms >= duration_ms`, set `fade = FadeState::Idle` and
///    `progress = target_opacity`.
/// 5. For `FadeState::Out`, approach 0.0 and despawn/hide the window at the end.
///
/// The `progress` field of `WindowAnimation` is already read by the render
/// compositor in `compose_frame_system` to set per-surface opacity.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FadeEffect {
    pub duration_ms: u32,
}

/// Global config for fade animations.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct FadeEffectConfig {
    pub enabled: bool,
    pub open_duration_ms: u32,
    pub close_duration_ms: u32,
}

impl Default for FadeEffectConfig {
    fn default() -> Self {
        Self { enabled: false, open_duration_ms: 200, close_duration_ms: 150 }
    }
}

/// Fade effect system — placeholder, not yet implemented.
///
/// When implemented this will advance `WindowAnimation::elapsed_ms` during the
/// pre-render schedule and update `progress`/`fade` before
/// `surface_visual_snapshot_system` projects the result for core rendering.
pub fn fade_effect_system() {
    // FUTURE: drive WindowAnimation::progress from FadeState — see FadeEffect doc above
    tracing::trace!("fade effect system tick (not yet implemented)");
}
