/// Protocol state for output-management support.
#[derive(Debug, Clone, Default)]
pub struct OutputManagementState;

impl OutputManagementState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwlr_output_manager_v1"]
    }
}
