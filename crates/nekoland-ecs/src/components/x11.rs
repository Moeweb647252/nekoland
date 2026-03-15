use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

/// Normalized X11 window-type classification copied out of XWayland metadata.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum X11WindowType {
    DropdownMenu,
    Dialog,
    Menu,
    Notification,
    Normal,
    PopupMenu,
    Splash,
    Toolbar,
    Tooltip,
    Utility,
}

/// Extra metadata attached to windows whose lifecycle is driven by XWayland/X11.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(crate::components::XdgWindow)]
pub struct X11Window {
    pub window_id: u32,
    pub override_redirect: bool,
    pub popup: bool,
    pub transient_for: Option<u32>,
    pub window_type: Option<X11WindowType>,
}

impl X11Window {
    pub fn is_helper_surface(&self) -> bool {
        self.popup
            || matches!(
                self.window_type,
                Some(
                    X11WindowType::DropdownMenu
                        | X11WindowType::Menu
                        | X11WindowType::Notification
                        | X11WindowType::PopupMenu
                        | X11WindowType::Tooltip
                )
            )
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::{X11Window, X11WindowType};
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

    #[test]
    fn x11_window_helper_detection_matches_popup_and_helper_types() {
        assert!(X11Window { popup: true, ..Default::default() }.is_helper_surface());
        assert!(
            X11Window { window_type: Some(X11WindowType::Tooltip), ..Default::default() }
                .is_helper_surface()
        );
        assert!(
            !X11Window { window_type: Some(X11WindowType::Dialog), ..Default::default() }
                .is_helper_surface()
        );
    }
}
