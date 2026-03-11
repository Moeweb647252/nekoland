use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputSeat {
    pub name: String,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PointerPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardFocus {
    pub surface_id: Option<u64>,
}
