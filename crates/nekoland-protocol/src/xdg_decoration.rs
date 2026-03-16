use crate::ProtocolGlobals;

/// Protocol state for XDG decoration negotiation.
#[derive(Debug, Clone, Default)]
pub struct XdgDecorationState;

impl ProtocolGlobals for XdgDecorationState {
    const GLOBALS: &'static [&'static str] = &["zxdg_decoration_manager_v1"];
}
