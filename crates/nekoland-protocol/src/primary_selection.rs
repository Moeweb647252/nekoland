#[derive(Debug, Clone, Default)]
pub struct PrimarySelectionProtocolState;

impl PrimarySelectionProtocolState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwp_primary_selection_device_manager_v1"]
    }
}
