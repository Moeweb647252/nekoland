use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Machine-word scene-space coordinate used for infinite workspace positioning.
pub type WorkspaceCoord = isize;

pub fn workspace_coord_from_i64(value: i64) -> WorkspaceCoord {
    value.clamp(isize::MIN as i64, isize::MAX as i64) as WorkspaceCoord
}

pub fn workspace_coord_to_i64(value: WorkspaceCoord) -> i64 {
    value.clamp(i64::MIN as WorkspaceCoord, i64::MAX as WorkspaceCoord) as i64
}

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

/// Monotonic counter bumped whenever the compositor observes a new content commit for a surface.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceContentVersion {
    pub value: u64,
}

impl SurfaceContentVersion {
    pub fn bump(&mut self) {
        self.value = self.value.saturating_add(1);
    }
}
