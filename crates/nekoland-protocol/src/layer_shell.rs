use crate::ProtocolGlobals;

/// Protocol state for wlr-layer-shell support.
#[derive(Debug, Clone, Default)]
pub struct LayerShellState;

impl ProtocolGlobals for LayerShellState {
    const GLOBALS: &'static [&'static str] = &["zwlr_layer_shell_v1"];
}
