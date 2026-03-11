#[derive(Debug, Clone, Default, PartialEq)]
pub struct RoundedCornerEffect {
    pub radius: f32,
}

pub fn rounded_corner_effect_system() {
    tracing::trace!("rounded corner effect system tick");
}
