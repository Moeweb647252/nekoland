use crate::ProtocolGlobals;

/// Protocol state for clipboard and drag-and-drop data-device support.
#[derive(Debug, Clone, Default)]
pub struct DataDeviceState;

impl ProtocolGlobals for DataDeviceState {
    const GLOBALS: &'static [&'static str] = &["wl_data_device_manager"];
}
