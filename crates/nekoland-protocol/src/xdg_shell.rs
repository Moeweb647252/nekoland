use crate::ProtocolGlobals;

/// Protocol state for XDG shell support.
#[derive(Debug, Clone, Default)]
pub struct XdgShellState;

impl ProtocolGlobals for XdgShellState {
    const GLOBALS: &'static [&'static str] = &["xdg_wm_base"];
}
