//! Optional user-facing visual effects kept separate from core compositor rendering.

use bevy_app::App;
use nekoland_core::plugin::NekolandPlugin;

pub mod blur;
pub mod fade;
pub mod rounded_corners;
pub mod shadow;

pub fn install_main_render_features(app: &mut App) {
    blur::BlurEffectPlugin::init_config(app);
    shadow::ShadowEffectPlugin::init_config(app);
    rounded_corners::RoundedCornerEffectPlugin::init_config(app);
    fade::FadeEffectPlugin.build(app);
}

pub fn install_render_subapp_features(app: &mut App) {
    blur::BlurEffectPlugin::install_render_subapp(app);
    shadow::ShadowEffectPlugin::install_render_subapp(app);
    rounded_corners::RoundedCornerEffectPlugin::install_render_subapp(app);
}
