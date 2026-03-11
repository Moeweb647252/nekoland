use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalPointerPosition {
    pub x: f64,
    pub y: f64,
}
