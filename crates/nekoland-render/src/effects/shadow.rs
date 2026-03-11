/// Shadow effect configuration.
///
/// # FUTURE: Implementation guide
///
/// When implementing drop shadows:
/// 1. In `shadow_effect_system`, read `Res<ShadowEffectConfig>`.
/// 2. For each visible window, emit a `ShadowRenderElement` behind the surface
///    element in the render list.
/// 3. The shadow element renders a blurred, offset, semi-transparent rectangle
///    using a custom Smithay render element implementation.
///
/// `spread` is the shadow blur radius in pixels; `offset` is the drop offset.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowEffect {
    pub spread: f32,
}

/// Global config for the shadow effect.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowEffectConfig {
    pub enabled: bool,
    pub spread: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub color: [f32; 4],
}

impl Default for ShadowEffectConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            spread: 12.0,
            offset_x: 0.0,
            offset_y: 4.0,
            color: [0.0, 0.0, 0.0, 0.5],
        }
    }
}

/// Shadow effect system — placeholder, not yet implemented.
///
/// When implemented this system will emit `ShadowRenderElement` behind each
/// visible window surface element.
pub fn shadow_effect_system() {
    // FUTURE: shadow rendering element — see ShadowEffect doc above
    tracing::trace!("shadow effect system tick (not yet implemented)");
}
