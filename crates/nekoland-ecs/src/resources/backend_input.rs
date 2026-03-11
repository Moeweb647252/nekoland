use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BackendInputAction {
    Key { keycode: u32, pressed: bool },
    PointerMoved { x: f64, y: f64 },
    PointerButton { button_code: u32, pressed: bool },
    PointerAxis { horizontal: f64, vertical: f64 },
    FocusChanged { focused: bool },
}

impl Default for BackendInputAction {
    fn default() -> Self {
        Self::FocusChanged { focused: true }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BackendInputEvent {
    pub device: String,
    pub action: BackendInputAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PendingBackendInputEvents {
    pub items: Vec<BackendInputEvent>,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PendingProtocolInputEvents {
    pub items: Vec<BackendInputEvent>,
}
