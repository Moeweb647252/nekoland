#[derive(Debug, Clone, Default)]
pub struct FractionalScaleState;

impl FractionalScaleState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wp_fractional_scale_manager_v1"]
    }
}
