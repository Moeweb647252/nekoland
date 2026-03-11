#[derive(Debug, Clone, Default)]
pub struct LayerShellState;

impl LayerShellState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwlr_layer_shell_v1"]
    }
}
