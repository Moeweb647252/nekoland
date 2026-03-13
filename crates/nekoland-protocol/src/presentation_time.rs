/// Protocol state for presentation-time feedback support.
#[derive(Debug, Clone, Default)]
pub struct PresentationTimeState;

impl PresentationTimeState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wp_presentation"]
    }
}
