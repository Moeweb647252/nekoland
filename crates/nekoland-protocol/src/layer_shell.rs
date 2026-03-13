/// Protocol state for wlr-layer-shell support.
#[derive(Debug, Clone, Default)]
pub struct LayerShellState;

impl LayerShellState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwlr_layer_shell_v1"]
    }
}
