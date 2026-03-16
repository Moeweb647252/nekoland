use crate::ProtocolGlobals;

/// Protocol state for presentation-time feedback support.
#[derive(Debug, Clone, Default)]
pub struct PresentationTimeState;

impl ProtocolGlobals for PresentationTimeState {
    const GLOBALS: &'static [&'static str] = &["wp_presentation"];
}
