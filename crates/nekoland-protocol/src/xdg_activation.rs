/// Protocol state for xdg-activation support.
#[derive(Debug, Clone, Default)]
pub struct XdgActivationState;

impl XdgActivationState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["xdg_activation_v1"]
    }
}
