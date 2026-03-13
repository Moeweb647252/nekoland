use bevy_ecs::prelude::Message;
use serde::{Deserialize, Serialize};

/// Keyboard key transition emitted by the input pipeline.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyPress {
    pub keycode: u32,
    pub pressed: bool,
}

/// Pointer motion emitted by the input pipeline.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PointerMotion {
    pub x: f64,
    pub y: f64,
}

/// Pointer button transition emitted by the input pipeline.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PointerButton {
    pub button_code: u32,
    pub pressed: bool,
}

/// High-level swipe gesture emitted by the gesture recognizer.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GestureSwipe {
    pub delta_x: f32,
    pub delta_y: f32,
    pub fingers: u8,
}

/// Notification that an external command was successfully launched.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandLaunched {
    pub origin: String,
    pub command: Vec<String>,
    pub pid: u32,
}

/// Notification that launching an external command failed.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandFailed {
    pub origin: String,
    pub candidates: Vec<Vec<String>>,
    pub error: String,
}
