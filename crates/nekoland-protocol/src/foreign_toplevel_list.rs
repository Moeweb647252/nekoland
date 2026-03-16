use crate::ProtocolGlobals;

/// Protocol state for ext-foreign-toplevel-list support.
#[derive(Debug, Clone, Default)]
pub struct ForeignToplevelListProtocolState;

impl ProtocolGlobals for ForeignToplevelListProtocolState {
    const GLOBALS: &'static [&'static str] = &["ext_foreign_toplevel_list_v1"];
}
