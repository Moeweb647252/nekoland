//! Window lifecycle messages emitted by shell policy systems.
#![allow(missing_docs)]

use bevy_ecs::prelude::Message;
use serde::{Deserialize, Serialize};

/// Notification that a new window entity was created.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowCreated {
    pub surface_id: u64,
    pub title: String,
}

/// Notification that a window entity was removed.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowClosed {
    pub surface_id: u64,
}

/// Notification that a window geometry move was applied.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowMoved {
    pub surface_id: u64,
    pub x: i64,
    pub y: i64,
}
