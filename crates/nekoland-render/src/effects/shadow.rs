#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShadowEffect {
    pub spread: f32,
}

pub fn shadow_effect_system() {
    tracing::trace!("shadow effect system tick");
}
