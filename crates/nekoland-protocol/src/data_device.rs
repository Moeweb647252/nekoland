#[derive(Debug, Clone, Default)]
pub struct DataDeviceState;

impl DataDeviceState {
    pub fn globals(&self) -> Vec<&'static str> {
        vec!["wl_data_device_manager"]
    }
}
