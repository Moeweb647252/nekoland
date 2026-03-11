use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct XdgPopup {
    pub parent_surface: u64,
    pub configure_serial: Option<u32>,
    pub grab_serial: Option<u32>,
    pub reposition_token: Option<u32>,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupGrab {
    pub active: bool,
    pub seat_name: String,
    pub serial: Option<u32>,
}
