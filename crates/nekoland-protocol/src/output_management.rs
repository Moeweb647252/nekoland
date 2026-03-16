use crate::ProtocolGlobals;

/// Protocol state for output-management support.
#[derive(Debug, Clone, Default)]
pub struct OutputManagementState;

impl ProtocolGlobals for OutputManagementState {
    const GLOBALS: &'static [&'static str] = &["zwlr_output_manager_v1"];
}
