use crate::ProtocolGlobals;

/// Protocol state for primary-selection support.
#[derive(Debug, Clone, Default)]
pub struct PrimarySelectionProtocolState;

impl ProtocolGlobals for PrimarySelectionProtocolState {
    const GLOBALS: &'static [&'static str] = &["zwp_primary_selection_device_manager_v1"];
}
