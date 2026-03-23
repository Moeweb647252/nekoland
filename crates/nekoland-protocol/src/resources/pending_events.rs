use std::fmt;

use nekoland_ecs::kinds::{BackendEventQueue, FrameQueue, ProtocolEventQueue};
use serde::{Deserialize, Serialize};

/// Distinguishes the XDG surface role associated with a lifecycle event.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum XdgSurfaceRole {
    #[default]
    Toplevel,
    Popup,
}

/// Buffer size reported by protocol commits.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceExtent {
    pub width: u32,
    pub height: u32,
}

/// Popup placement geometry copied from protocol state into the ECS request queue.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub reposition_token: Option<u32>,
}

/// Normalized interactive resize edge selection shared across protocol and shell layers.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResizeEdges {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeEdges {
    pub fn has_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    pub fn has_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    pub fn has_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    pub fn has_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

impl fmt::Display for ResizeEdges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::TopLeft => "top_left",
            Self::TopRight => "top_right",
            Self::BottomLeft => "bottom_left",
            Self::BottomRight => "bottom_right",
        };
        f.write_str(name)
    }
}

/// Window and popup lifecycle actions buffered between protocol callbacks and shell systems.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowLifecycleAction {
    Committed { role: XdgSurfaceRole, size: Option<SurfaceExtent> },
    ConfigureRequested { role: XdgSurfaceRole },
    AckConfigure { role: XdgSurfaceRole, serial: u32 },
    MetadataChanged { title: Option<String>, app_id: Option<String> },
    InteractiveMove { seat_name: String, serial: u32 },
    InteractiveResize { seat_name: String, serial: u32, edges: ResizeEdges },
    Maximize,
    UnMaximize,
    Fullscreen { output_name: Option<String> },
    UnFullscreen,
    Minimize,
    PopupCreated { parent_surface_id: Option<u64>, placement: PopupPlacement },
    PopupRepositioned { placement: PopupPlacement },
    PopupGrab { seat_name: String, serial: u32 },
    Destroyed { role: XdgSurfaceRole },
}

impl Default for WindowLifecycleAction {
    fn default() -> Self {
        Self::Committed { role: XdgSurfaceRole::Toplevel, size: None }
    }
}

/// One queued lifecycle request targeting a surface id.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowLifecycleRequest {
    pub surface_id: u64,
    pub action: WindowLifecycleAction,
}

/// Protocol-to-shell queue for XDG lifecycle events.
pub type PendingXdgRequests = ProtocolEventQueue<WindowLifecycleRequest>;

/// Human-readable input log entry used by tests and diagnostics.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputEventRecord {
    pub source: String,
    pub detail: String,
}

/// Buffered input log records collected during the current frame.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingInputEventsTag;

pub type PendingInputEvents = FrameQueue<InputEventRecord, PendingInputEventsTag>;

/// Coarse output lifecycle record emitted by backends and protocol bridges.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputEventRecord {
    pub output_name: String,
    pub change: String,
}

/// Buffered output lifecycle records collected during the current frame.
pub type PendingOutputEvents = BackendEventQueue<OutputEventRecord>;
