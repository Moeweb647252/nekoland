use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Stable compositor-assigned id used to correlate entities with protocol/back-end surfaces.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WlSurfaceHandle {
    pub id: u64,
}

/// Surface rectangle in compositor-global coordinates.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Minimal buffer attachment state used by layout and render decisions.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BufferState {
    pub attached: bool,
    pub scale: i32,
}
