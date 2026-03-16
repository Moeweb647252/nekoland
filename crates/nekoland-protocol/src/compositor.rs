use crate::ProtocolGlobals;

/// Protocol state for the core compositor globals.
#[derive(Debug, Clone, Default)]
pub struct CompositorProtocolState;

impl ProtocolGlobals for CompositorProtocolState {
    const GLOBALS: &'static [&'static str] = &["wl_compositor", "wl_subcompositor"];
}
