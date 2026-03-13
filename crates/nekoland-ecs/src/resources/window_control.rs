use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::selectors::{SurfaceId, WindowSelector};

use super::{KeyboardFocusState, SplitAxis};

/// High-level window control updates staged by IPC, keybindings, or other systems.
///
/// Callers operate on one logical window control object instead of hand-assembling several
/// low-level transport requests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWindowControls {
    controls: Vec<PendingWindowControl>,
}

/// One staged control update for a single window surface.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWindowControl {
    pub surface_id: SurfaceId,
    pub position: Option<WindowControlPosition>,
    pub size: Option<WindowControlSize>,
    pub split_axis: Option<SplitAxis>,
    pub focus: bool,
    pub close: bool,
}

/// Desired top-left origin to apply to the target window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowControlPosition {
    pub x: i32,
    pub y: i32,
}

/// Desired size to apply to the target window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowControlSize {
    pub width: u32,
    pub height: u32,
}

/// Mutable façade over one staged control entry.
pub struct WindowControlHandle<'a> {
    control: &'a mut PendingWindowControl,
}

impl PendingWindowControls {
    /// Queue-or-reuse the staged control entry for a specific surface id.
    pub fn surface(&mut self, surface_id: SurfaceId) -> WindowControlHandle<'_> {
        let index = self.controls.iter().position(|control| control.surface_id == surface_id);
        let control = if let Some(index) = index {
            &mut self.controls[index]
        } else {
            self.controls
                .push(PendingWindowControl { surface_id, ..PendingWindowControl::default() });
            self.controls.last_mut().expect("window control just pushed")
        };

        WindowControlHandle { control }
    }

    /// Queue-or-reuse the staged control entry for a typed window selector.
    pub fn select(
        &mut self,
        selector: WindowSelector,
        keyboard_focus: &KeyboardFocusState,
    ) -> Option<WindowControlHandle<'_>> {
        match selector {
            WindowSelector::Focused => self.focused(keyboard_focus),
            WindowSelector::Surface(surface_id) => Some(self.surface(surface_id)),
        }
    }

    /// Returns a mutable control handle for the currently focused surface, if any.
    pub fn focused(
        &mut self,
        keyboard_focus: &KeyboardFocusState,
    ) -> Option<WindowControlHandle<'_>> {
        keyboard_focus.focused_surface.map(|surface_id| self.surface(SurfaceId(surface_id)))
    }

    /// Drain all staged controls in insertion order.
    pub fn take(&mut self) -> Vec<PendingWindowControl> {
        std::mem::take(&mut self.controls)
    }

    /// Replace the staged controls wholesale, typically to defer unresolved targets.
    pub fn replace(&mut self, controls: Vec<PendingWindowControl>) {
        self.controls = controls;
    }

    /// Inspect all staged controls without consuming them.
    pub fn as_slice(&self) -> &[PendingWindowControl] {
        &self.controls
    }

    /// Remove all staged controls.
    pub fn clear(&mut self) {
        self.controls.clear();
    }

    /// Return whether any staged controls remain.
    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

impl WindowControlHandle<'_> {
    /// Stage a floating move for the target window.
    pub fn move_to(&mut self, x: i32, y: i32) -> &mut Self {
        self.control.position = Some(WindowControlPosition { x, y });
        self
    }

    /// Stage a floating resize for the target window.
    pub fn resize_to(&mut self, width: u32, height: u32) -> &mut Self {
        self.control.size = Some(WindowControlSize { width, height });
        self
    }

    /// Stage a tiled split-axis update for the target window.
    pub fn split(&mut self, axis: SplitAxis) -> &mut Self {
        self.control.split_axis = Some(axis);
        self
    }

    /// Stage a horizontal split for the target window.
    pub fn split_horizontal(&mut self) -> &mut Self {
        self.split(SplitAxis::Horizontal)
    }

    /// Stage a vertical split for the target window.
    pub fn split_vertical(&mut self) -> &mut Self {
        self.split(SplitAxis::Vertical)
    }

    /// Stage focus for the target window.
    pub fn focus(&mut self) -> &mut Self {
        self.control.focus = true;
        self
    }

    /// Stage close for the target window.
    pub fn close(&mut self) -> &mut Self {
        self.control.close = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::resources::KeyboardFocusState;
    use crate::resources::SplitAxis;
    use crate::selectors::SurfaceId;

    use super::PendingWindowControls;

    #[test]
    fn surface_controls_merge_move_resize_focus_and_close() {
        let mut controls = PendingWindowControls::default();
        controls
            .surface(SurfaceId(7))
            .move_to(10, 20)
            .resize_to(800, 600)
            .split_vertical()
            .focus()
            .close();

        assert_eq!(controls.as_slice().len(), 1);
        let control = &controls.as_slice()[0];
        assert_eq!(control.surface_id, SurfaceId(7));
        assert_eq!(control.position.expect("position").x, 10);
        assert_eq!(control.position.expect("position").y, 20);
        assert_eq!(control.size.expect("size").width, 800);
        assert_eq!(control.size.expect("size").height, 600);
        assert_eq!(control.split_axis, Some(SplitAxis::Vertical));
        assert!(control.focus);
        assert!(control.close);
    }

    #[test]
    fn focused_uses_keyboard_focus_surface() {
        let mut controls = PendingWindowControls::default();
        let focus = KeyboardFocusState { focused_surface: Some(42) };
        controls.focused(&focus).expect("focused window").close();

        assert_eq!(controls.as_slice()[0].surface_id, SurfaceId(42));
        assert!(controls.as_slice()[0].close);
    }
}
