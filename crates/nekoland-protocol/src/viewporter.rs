/// Protocol state for wp_viewporter support.
#[derive(Debug, Clone, Default)]
pub struct ViewporterState;

impl ViewporterState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wp_viewporter"]
    }
}
