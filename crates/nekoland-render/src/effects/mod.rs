//! Optional user-facing visual effects kept separate from core compositor rendering.

use bevy_app::App;
use nekoland_core::plugin::NekolandPlugin;

/// Blur effect feature plugin and typed materials.
pub mod blur;
/// Fade-animation bridge from shell animation state into render timelines.
pub mod fade;
/// Rounded-corner effect feature plugin and typed materials.
pub mod rounded_corners;
/// Shadow effect feature plugin and typed materials.
pub mod shadow;

/// Installs effect configuration and main-world hooks required before render extraction.
pub fn install_main_render_features(app: &mut App) {
    blur::BlurEffectPlugin::init_config(app);
    shadow::ShadowEffectPlugin::init_config(app);
    rounded_corners::RoundedCornerEffectPlugin::init_config(app);
    fade::FadeEffectPlugin.build(app);
}

/// Installs render-subapp systems for all enabled render features.
pub fn install_render_subapp_features(app: &mut App) {
    blur::BlurEffectPlugin::install_render_subapp(app);
    shadow::ShadowEffectPlugin::install_render_subapp(app);
    rounded_corners::RoundedCornerEffectPlugin::install_render_subapp(app);
}
