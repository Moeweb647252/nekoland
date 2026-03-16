use crate::ProtocolGlobals;

/// Protocol state for fractional-scale support.
#[derive(Debug, Clone, Default)]
pub struct FractionalScaleState;

impl ProtocolGlobals for FractionalScaleState {
    const GLOBALS: &'static [&'static str] = &["wp_fractional_scale_manager_v1"];
}
