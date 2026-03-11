/// Blur effect configuration.
///
/// # FUTURE: Implementation guide
///
/// When implementing GPU-side blur:
/// 1. In `blur_effect_system`, read `Res<BlurEffectConfig>` to check `enabled`.
/// 2. For each surface in the `RenderList`, render a downsampled, blurred
///    version of the background into a separate framebuffer.
/// 3. Composite the blurred framebuffer behind the surface using Smithay's
///    render element API.
/// 4. Consider using `DualKawase` or two-pass Gaussian for performance.
///
/// `radius` controls the blur kernel size in pixels.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct BlurEffect {
    pub radius: f32,
}

/// Global config for the blur effect (read by the system when implemented).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct BlurEffectConfig {
    pub enabled: bool,
    pub radius: f32,
}

impl Default for BlurEffectConfig {
    fn default() -> Self {
        Self { enabled: false, radius: 8.0 }
    }
}

/// Blur effect system — placeholder, not yet implemented.
///
/// When implemented this system will:
/// - Read `Res<BlurEffectConfig>` to determine if blur is active.
/// - Render behind-window samples into an offscreen buffer.
/// - Composite the result during the render pass.
pub fn blur_effect_system() {
    // FUTURE: GPU-side blur pass — see BlurEffect doc above
    tracing::trace!("blur effect system tick (not yet implemented)");
}
