use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Stored protocol state for an XDG popup surface.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(
    crate::components::SurfaceGeometry,
    crate::components::BufferState,
    crate::components::SurfaceContentVersion,
    PopupGrab,
    crate::components::WindowAnimation
)]
pub struct XdgPopup {
    pub configure_serial: Option<u32>,
    pub grab_serial: Option<u32>,
    pub reposition_token: Option<u32>,
    pub placement_x: i32,
    pub placement_y: i32,
    pub placement_width: u32,
    pub placement_height: u32,
}

/// Tracks whether a popup currently owns an explicit popup grab.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupGrab {
    pub active: bool,
    pub seat_name: String,
    pub serial: Option<u32>,
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::{PopupGrab, XdgPopup};
    use crate::components::{BufferState, SurfaceGeometry, WindowAnimation};

    #[test]
    fn xdg_popup_requires_surface_runtime_components() {
        let mut world = World::new();
        let entity = world.spawn(XdgPopup::default()).id();

        assert!(world.get::<SurfaceGeometry>(entity).is_some());
        assert!(world.get::<BufferState>(entity).is_some());
        assert!(world.get::<PopupGrab>(entity).is_some());
        assert!(world.get::<WindowAnimation>(entity).is_some());
    }
}
