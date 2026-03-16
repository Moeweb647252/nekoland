use crate::ProtocolGlobals;

/// Protocol state for wp_viewporter support.
#[derive(Debug, Clone, Default)]
pub struct ViewporterState;

impl ProtocolGlobals for ViewporterState {
    const GLOBALS: &'static [&'static str] = &["wp_viewporter"];
}
