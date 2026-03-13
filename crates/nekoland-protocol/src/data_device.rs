/// Protocol state for clipboard and drag-and-drop data-device support.
#[derive(Debug, Clone, Default)]
pub struct DataDeviceState;

impl DataDeviceState {
    /// Returns the globals this protocol module expects the Smithay runtime to advertise.
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wl_data_device_manager"]
    }
}
