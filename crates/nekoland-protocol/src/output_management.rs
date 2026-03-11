#[derive(Debug, Clone, Default)]
pub struct OutputManagementState;

impl OutputManagementState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwlr_output_manager_v1"]
    }
}
