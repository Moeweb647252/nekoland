use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Extra metadata attached to windows whose lifecycle is driven by XWayland/X11.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(crate::components::XdgWindow)]
pub struct X11Window {
    pub window_id: u32,
    pub override_redirect: bool,
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::X11Window;
    use crate::components::{BufferState, SurfaceGeometry, WindowAnimation, WindowMode, XdgWindow};

    #[test]
    fn x11_window_requires_xdg_window_stack() {
        let mut world = World::new();
        let entity = world.spawn(X11Window::default()).id();

        assert!(world.get::<XdgWindow>(entity).is_some());
        assert!(world.get::<SurfaceGeometry>(entity).is_some());
        assert!(world.get::<BufferState>(entity).is_some());
        assert!(world.get::<WindowMode>(entity).is_some());
        assert!(world.get::<WindowAnimation>(entity).is_some());
    }
}
