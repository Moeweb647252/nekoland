#[derive(Debug, Clone, Default, PartialEq)]
pub struct FadeEffect {
    pub duration_ms: u32,
}

pub fn fade_effect_system() {
    tracing::trace!("fade effect system tick");
}
