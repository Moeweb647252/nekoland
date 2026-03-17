use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

use crate::components::WorkspaceCoord;
use crate::selectors::OutputName;

/// Metadata tracked for a mapped XDG toplevel surface.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(
    crate::components::SurfaceGeometry,
    crate::components::BufferState,
    crate::components::SurfaceContentVersion,
    WindowSceneGeometry,
    WindowViewportVisibility,
    WindowRole,
    WindowLayout,
    WindowMode,
    WindowFullscreenTarget,
    WindowPolicyState,
    WindowPlacement,
    WindowRestoreSnapshot,
    crate::components::WindowAnimation
)]
pub struct XdgWindow {
    pub app_id: String,
    pub title: String,
    pub last_acked_configure: Option<u32>,
}

/// Authoritative window geometry in workspace-scene coordinates.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowSceneGeometry {
    pub x: WorkspaceCoord,
    pub y: WorkspaceCoord,
    pub width: u32,
    pub height: u32,
}

/// Whether the current window scene geometry intersects the active output viewport.
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowViewportVisibility {
    pub visible: bool,
    pub output: Option<String>,
}

impl Default for WindowViewportVisibility {
    fn default() -> Self {
        Self { visible: true, output: None }
    }
}

/// Explicit runtime role for one managed window entity.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowRole {
    #[default]
    Managed,
    OutputBackground,
}

impl WindowRole {
    pub const fn is_managed(self) -> bool {
        matches!(self, Self::Managed)
    }

    pub const fn is_output_background(self) -> bool {
        matches!(self, Self::OutputBackground)
    }
}

/// User-facing floating placement hints for a window.
///
/// This is deliberately higher-level than `SurfaceGeometry`: control-plane code updates placement
/// intent here, and layout systems reconcile it into the current geometry/result each frame.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowPlacement {
    pub floating_position: Option<FloatingPosition>,
    pub floating_size: Option<WindowSize>,
}

/// Floating-position source tracked for one window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FloatingPosition {
    Auto(WindowPosition),
    Explicit(WindowPosition),
}

/// Per-window geometry policy selected by the window manager.
///
/// Stacking is handled separately through z-order state; layout only decides how systems derive
/// or constrain `SurfaceGeometry`.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WindowLayout {
    #[default]
    Tiled,
    Floating,
}

impl WindowLayout {
    pub const fn border_width(self) -> u32 {
        match self {
            Self::Tiled => 2,
            Self::Floating => 1,
        }
    }
}

/// Desired top-left origin for a floating window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowPosition {
    pub x: WorkspaceCoord,
    pub y: WorkspaceCoord,
}

/// Desired size for a floating window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl WindowPlacement {
    pub fn resolved_floating_position(&self) -> Option<WindowPosition> {
        self.floating_position.map(FloatingPosition::position)
    }

    pub fn set_auto_position(&mut self, position: WindowPosition) {
        self.floating_position = Some(FloatingPosition::Auto(position));
    }

    pub fn set_explicit_position(&mut self, position: WindowPosition) {
        self.floating_position = Some(FloatingPosition::Explicit(position));
    }

    pub fn has_explicit_placement(&self) -> bool {
        self.floating_position.is_some_and(FloatingPosition::is_explicit)
            || self.floating_size.is_some()
    }

    pub fn should_auto_place(&self, geometry: &WindowSceneGeometry) -> bool {
        self.floating_size.is_none()
            && self.floating_position.is_none()
            && geometry.x == 0
            && geometry.y == 0
    }

    pub fn should_reposition_auto(
        &self,
        geometry: &WindowSceneGeometry,
        work_area_changed: bool,
    ) -> bool {
        if self.floating_size.is_some() {
            return false;
        }

        match self.floating_position {
            None => geometry.x == 0 && geometry.y == 0,
            Some(FloatingPosition::Auto(_)) => work_area_changed,
            Some(FloatingPosition::Explicit(_)) => false,
        }
    }
}

impl FloatingPosition {
    pub fn position(self) -> WindowPosition {
        match self {
            Self::Auto(position) | Self::Explicit(position) => position,
        }
    }

    pub fn is_explicit(self) -> bool {
        matches!(self, Self::Explicit(_))
    }
}

/// Geometry/layout/mode snapshot used to restore a window after temporary mode overrides such as
/// maximize or fullscreen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowRestoreState {
    pub geometry: WindowSceneGeometry,
    pub layout: WindowLayout,
    pub mode: WindowMode,
    #[serde(default)]
    pub fullscreen_output: Option<OutputName>,
}

/// Per-window restore snapshot storage.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowRestoreSnapshot {
    pub snapshot: Option<WindowRestoreState>,
}

/// Optional output target used while a managed window is in fullscreen mode.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowFullscreenTarget {
    pub output: Option<OutputName>,
}

/// Output-scoped background role that removes a window from the normal workspace scene.
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputBackgroundWindow {
    pub output: String,
    pub restore: WindowRestoreState,
}

/// Typed window policy resolved from config defaults plus matching window rules.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowPolicy {
    pub layout: WindowLayout,
    pub mode: WindowMode,
}

impl WindowPolicy {
    pub const fn new(layout: WindowLayout, mode: WindowMode) -> Self {
        Self { layout, mode }
    }

    pub fn apply(self, layout: &mut WindowLayout, mode: &mut WindowMode) {
        *layout = self.layout;
        *mode = self.mode;
    }
}

/// Tracks the currently applied default policy for one window and whether later metadata updates
/// may still refresh it.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowPolicyState {
    pub applied: WindowPolicy,
    pub locked: bool,
}

impl WindowPolicyState {
    pub fn tracks_current(&self, layout: WindowLayout, mode: WindowMode) -> bool {
        !self.locked && self.applied == WindowPolicy::new(layout, mode)
    }
}

/// High-level presentation mode layered on top of the base layout geometry.
#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    #[default]
    Normal,
    Maximized,
    Fullscreen,
    Hidden,
}

/// Read-only user-facing state derived from `WindowLayout` and `WindowMode`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowDisplayState {
    Tiled,
    Floating,
    Maximized,
    Fullscreen,
    Hidden,
}

impl WindowDisplayState {
    pub fn from_layout_mode(layout: WindowLayout, mode: WindowMode) -> Self {
        match mode {
            WindowMode::Normal => match layout {
                WindowLayout::Floating => Self::Floating,
                WindowLayout::Tiled => Self::Tiled,
            },
            WindowMode::Maximized => Self::Maximized,
            WindowMode::Fullscreen => Self::Fullscreen,
            WindowMode::Hidden => Self::Hidden,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tiled => "Tiled",
            Self::Floating => "Floating",
            Self::Maximized => "Maximized",
            Self::Fullscreen => "Fullscreen",
            Self::Hidden => "Hidden",
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;

    use super::XdgWindow;
    use crate::components::{
        BufferState, SurfaceGeometry, WindowAnimation, WindowLayout, WindowMode, WindowPlacement,
        WindowPolicyState, WindowRestoreSnapshot, WindowSceneGeometry, WindowViewportVisibility,
    };

    #[test]
    fn xdg_window_requires_surface_runtime_components() {
        let mut world = World::new();
        let entity = world.spawn(XdgWindow::default()).id();

        assert!(world.get::<SurfaceGeometry>(entity).is_some());
        assert!(world.get::<WindowSceneGeometry>(entity).is_some());
        assert!(world.get::<WindowViewportVisibility>(entity).is_some());
        assert!(world.get::<BufferState>(entity).is_some());
        assert!(world.get::<WindowMode>(entity).is_some());
        assert!(world.get::<WindowLayout>(entity).is_some());
        assert!(world.get::<WindowPolicyState>(entity).is_some());
        assert!(world.get::<WindowPlacement>(entity).is_some());
        assert!(world.get::<WindowRestoreSnapshot>(entity).is_some());
        assert!(world.get::<WindowAnimation>(entity).is_some());
    }
}
