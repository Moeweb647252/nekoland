#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlurEffect {
    pub radius: f32,
}

pub fn blur_effect_system() {
    tracing::trace!("blur effect system tick");
}
