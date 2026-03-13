/// Protocol state for fractional-scale support.
#[derive(Debug, Clone, Default)]
pub struct FractionalScaleState;

impl FractionalScaleState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wp_fractional_scale_manager_v1"]
    }
}
