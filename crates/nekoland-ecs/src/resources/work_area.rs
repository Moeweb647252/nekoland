use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for WorkArea {
    fn default() -> Self {
        Self { x: 0, y: 0, width: 1280, height: 720 }
    }
}
