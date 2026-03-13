use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Global pointer position shared across input, focus, and virtual-output capture systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalPointerPosition {
    pub x: f64,
    pub y: f64,
}
