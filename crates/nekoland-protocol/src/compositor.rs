/// Protocol state for the core compositor globals.
#[derive(Debug, Clone, Default)]
pub struct CompositorProtocolState;

impl CompositorProtocolState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wl_compositor", "wl_subcompositor"]
    }
}
