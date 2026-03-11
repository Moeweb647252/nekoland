use bevy_ecs::prelude::Message;
use serde::{Deserialize, Serialize};

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowCreated {
    pub surface_id: u64,
    pub title: String,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowClosed {
    pub surface_id: u64,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowMoved {
    pub surface_id: u64,
    pub x: i32,
    pub y: i32,
}
