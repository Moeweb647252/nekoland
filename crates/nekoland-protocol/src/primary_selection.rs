/// Protocol state for primary-selection support.
#[derive(Debug, Clone, Default)]
pub struct PrimarySelectionProtocolState;

impl PrimarySelectionProtocolState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["zwp_primary_selection_device_manager_v1"]
    }
}
