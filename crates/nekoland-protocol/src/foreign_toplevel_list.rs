/// Protocol state for ext-foreign-toplevel-list support.
#[derive(Debug, Clone, Default)]
pub struct ForeignToplevelListProtocolState;

impl ForeignToplevelListProtocolState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["ext_foreign_toplevel_list_v1"]
    }
}
