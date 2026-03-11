use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowServerAction {
    Focus,
    Close,
    Move { x: i32, y: i32 },
    Resize { width: u32, height: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowServerRequest {
    pub surface_id: u64,
    pub action: WindowServerAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWindowServerRequests {
    pub items: Vec<WindowServerRequest>,
}
