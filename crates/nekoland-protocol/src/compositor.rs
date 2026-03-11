#[derive(Debug, Clone, Default)]
pub struct CompositorProtocolState;

impl CompositorProtocolState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wl_compositor", "wl_subcompositor"]
    }
}
