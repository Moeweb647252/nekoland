use bevy_ecs::prelude::Message;
use serde::{Deserialize, Serialize};

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyPress {
    pub keycode: u32,
    pub pressed: bool,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PointerMotion {
    pub x: f64,
    pub y: f64,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GestureSwipe {
    pub delta_x: f32,
    pub delta_y: f32,
    pub fingers: u8,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandLaunched {
    pub origin: String,
    pub command: Vec<String>,
    pub pid: u32,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandFailed {
    pub origin: String,
    pub candidates: Vec<Vec<String>>,
    pub error: String,
}
