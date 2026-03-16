use crate::ProtocolGlobals;

/// Protocol state for xdg-activation support.
#[derive(Debug, Clone, Default)]
pub struct XdgActivationState;

impl ProtocolGlobals for XdgActivationState {
    const GLOBALS: &'static [&'static str] = &["xdg_activation_v1"];
}
