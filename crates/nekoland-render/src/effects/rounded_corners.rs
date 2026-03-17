/// Rounded corner effect configuration.
///
/// # FUTURE: Implementation guide
///
/// When implementing rounded corners:
/// 1. Implement a custom Smithay `RenderElement` that draws a window surface
///    with a corner-masking shader (e.g. SDF circle mask in the fragment shader).
/// 2. In `rounded_corner_effect_system`, wrap each relevant `RenderPlan` surface item in the
///    custom element before backend presentation.
/// 3. Read `Res<RoundedCornerEffectConfig>` to determine the active radius and
///    whether the effect is enabled.
///
/// `radius` is the corner radius in logical pixels.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundedCornerEffect {
    pub radius: f32,
}

/// Global config for rounded corners.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct RoundedCornerEffectConfig {
    pub enabled: bool,
    pub radius: f32,
}

impl Default for RoundedCornerEffectConfig {
    fn default() -> Self {
        Self { enabled: false, radius: 8.0 }
    }
}

/// Rounded corner effect system — placeholder, not yet implemented.
///
/// When implemented this will wrap surface render elements in a corner-masking
/// custom element before they reach the frame compositor.
pub fn rounded_corner_effect_system() {
    // FUTURE: rounded corner shader element — see RoundedCornerEffect doc above
    tracing::trace!("rounded corner effect system tick (not yet implemented)");
}
