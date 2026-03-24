use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

use crate::components::SeatId;

/// Shell-facing popup state shared by Wayland and XWayland popup surfaces.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(
    crate::components::SurfaceGeometry,
    crate::components::BufferState,
    crate::components::SurfaceContentVersion,
    PopupGrab,
    crate::components::WindowAnimation
)]
pub struct PopupSurface {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Tracks whether a popup currently owns an explicit popup grab.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupGrab {
    pub active: bool,
    pub seat_id: SeatId,
    pub serial: Option<u32>,
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::{PopupGrab, PopupSurface};
    use crate::components::{BufferState, SurfaceGeometry, WindowAnimation};

    #[test]
    fn popup_surface_requires_surface_runtime_components() {
        let mut world = World::new();
        let entity = world.spawn(PopupSurface::default()).id();

        assert!(world.get::<SurfaceGeometry>(entity).is_some());
        assert!(world.get::<BufferState>(entity).is_some());
        assert!(world.get::<PopupGrab>(entity).is_some());
        assert!(world.get::<WindowAnimation>(entity).is_some());
    }
}
