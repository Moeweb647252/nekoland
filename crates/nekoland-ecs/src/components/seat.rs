use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Named logical input seat entity.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputSeat {
    pub name: String,
}

/// Pointer position component for seat entities or related input state entities.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PointerPosition {
    pub x: f64,
    pub y: f64,
}

/// Focus target associated with a seat.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardFocus {
    pub surface_id: Option<u64>,
}
