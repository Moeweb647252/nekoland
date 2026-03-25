use std::fmt;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::{
    LayerAnchor, LayerLevel, LayerMargins, SeatId, SurfaceGeometry, WindowManagementHints,
    WindowSceneGeometry, X11WindowType,
};
use crate::kinds::{
    BackendEvent, CompositorRequest, FrameQueue, ProtocolEvent, ProtocolEventQueue,
};

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

/// Popup lifecycle actions buffered between platform callbacks and shell systems.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PopupEvent {
    Created { parent_surface_id: u64, placement: PopupPlacement },
    Repositioned { placement: PopupPlacement },
    Committed { size: Option<SurfaceExtent>, attached: bool },
    Grab { seat_id: SeatId, serial: u32 },
    Closed,
}

/// One queued popup event targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupEventRequest {
    pub surface_id: u64,
    pub action: PopupEvent,
}

impl ProtocolEvent for PopupEventRequest {}

/// Platform-to-shell queue for popup lifecycle events.
pub type PendingPopupEvents = ProtocolEventQueue<PopupEventRequest>;

/// Normalized interactive resize edge selection shared across platform and shell layers.
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

/// Window and popup lifecycle actions buffered between platform callbacks and shell systems.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowLifecycleAction {
    Committed { role: XdgSurfaceRole, size: Option<SurfaceExtent> },
    ConfigureRequested { role: XdgSurfaceRole },
    AckConfigure { role: XdgSurfaceRole, serial: u32 },
    MetadataChanged { title: Option<String>, app_id: Option<String> },
    InteractiveMove { seat_id: SeatId, serial: u32 },
    InteractiveResize { seat_id: SeatId, serial: u32, edges: ResizeEdges },
    Maximize,
    UnMaximize,
    Fullscreen { output_name: Option<String> },
    UnFullscreen,
    Minimize,
    PopupCreated { parent_surface_id: Option<u64>, placement: PopupPlacement },
    PopupRepositioned { placement: PopupPlacement },
    PopupGrab { seat_id: SeatId, serial: u32 },
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

impl ProtocolEvent for WindowLifecycleRequest {}

/// Platform-to-shell queue for XDG lifecycle events.
pub type PendingXdgRequests = ProtocolEventQueue<WindowLifecycleRequest>;

/// High-level manager-side requests that the shell can apply uniformly to any managed window.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowManagerRequest {
    BeginMove,
    BeginResize { edges: ResizeEdges },
    Maximize,
    UnMaximize,
    Fullscreen { output_name: Option<String> },
    UnFullscreen,
    Minimize,
    UnMinimize,
}

/// Unified window events exported by the wayland subapp into the main app.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowEvent {
    Upsert {
        title: Option<String>,
        app_id: Option<String>,
        hints: WindowManagementHints,
        scene_geometry: Option<WindowSceneGeometry>,
        attached: bool,
    },
    Committed {
        size: Option<SurfaceExtent>,
        attached: bool,
    },
    ManagerRequest(WindowManagerRequest),
    Closed,
}

/// One queued window event targeting a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowEventRequest {
    pub surface_id: u64,
    pub action: WindowEvent,
}

impl ProtocolEvent for WindowEventRequest {}

/// Platform-to-shell queue for unified window lifecycle events.
pub type PendingWindowEvents = ProtocolEventQueue<WindowEventRequest>;

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

/// Coarse output lifecycle record emitted by backends and platform bridges.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputEventRecord {
    pub output_name: String,
    pub change: String,
}

impl BackendEvent for OutputEventRecord {}

/// Buffered output lifecycle records collected during the current frame.
pub type PendingOutputEvents = crate::kinds::BackendEventQueue<OutputEventRecord>;

/// Payload needed to create a layer-shell entity from a platform request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerSurfaceCreateSpec {
    pub namespace: String,
    pub output_name: Option<String>,
    pub layer: LayerLevel,
    pub anchor: LayerAnchor,
    pub desired_width: u32,
    pub desired_height: u32,
    pub exclusive_zone: i32,
    pub margins: LayerMargins,
}

/// Layer-shell lifecycle actions buffered between platform callbacks and shell systems.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerLifecycleAction {
    Created {
        spec: LayerSurfaceCreateSpec,
    },
    Committed {
        size: Option<SurfaceExtent>,
        anchor: LayerAnchor,
        desired_width: u32,
        desired_height: u32,
        exclusive_zone: i32,
        margins: LayerMargins,
    },
    Destroyed,
}

/// One layer lifecycle request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerLifecycleRequest {
    pub surface_id: u64,
    pub action: LayerLifecycleAction,
}

impl ProtocolEvent for LayerLifecycleRequest {}

/// Queue of pending layer-shell lifecycle requests.
pub type PendingLayerRequests = ProtocolEventQueue<LayerLifecycleRequest>;

/// Geometry reported for one X11 window.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct X11WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<(i32, i32, SurfaceExtent)> for X11WindowGeometry {
    fn from((x, y, size): (i32, i32, SurfaceExtent)) -> Self {
        Self { x, y, width: size.width, height: size.height }
    }
}

/// X11/XWayland lifecycle actions buffered before the shell bridge applies them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum X11LifecycleAction {
    Mapped {
        window_id: u32,
        override_redirect: bool,
        popup: bool,
        transient_for: Option<u32>,
        window_type: Option<X11WindowType>,
        title: String,
        app_id: String,
        geometry: X11WindowGeometry,
    },
    Reconfigured {
        title: String,
        app_id: String,
        popup: bool,
        transient_for: Option<u32>,
        window_type: Option<X11WindowType>,
        geometry: X11WindowGeometry,
    },
    Maximize,
    UnMaximize,
    Fullscreen,
    UnFullscreen,
    Minimize,
    UnMinimize,
    InteractiveMove {
        button: u32,
    },
    InteractiveResize {
        button: u32,
        edges: ResizeEdges,
    },
    Unmapped,
    Destroyed,
}

/// One X11 lifecycle request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct X11LifecycleRequest {
    pub surface_id: u64,
    pub action: X11LifecycleAction,
}

impl ProtocolEvent for X11LifecycleRequest {}

/// Queue of pending X11 lifecycle requests.
pub type PendingX11Requests = ProtocolEventQueue<X11LifecycleRequest>;

/// Internal platform bridge requests for windows.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowServerAction {
    Close,
    SyncPresentation {
        geometry: SurfaceGeometry,
        scene_geometry: Option<WindowSceneGeometry>,
        fullscreen: bool,
        maximized: bool,
        resizing: bool,
    },
    SyncXdgToplevelState {
        size: Option<SurfaceExtent>,
        fullscreen: bool,
        maximized: bool,
        resizing: bool,
    },
    SyncX11WindowPresentation {
        geometry: X11WindowGeometry,
        fullscreen: bool,
        maximized: bool,
    },
}

/// One low-level window request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowServerRequest {
    pub surface_id: u64,
    pub action: WindowServerAction,
}

impl CompositorRequest for WindowServerRequest {}

/// Queue of pending platform-bridge window requests.
pub type PendingWindowServerRequests = crate::kinds::CompositorRequestQueue<WindowServerRequest>;

/// Popup-management actions emitted by shell systems.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PopupServerAction {
    Dismiss,
}

/// One popup-management request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupServerRequest {
    pub surface_id: u64,
    pub action: PopupServerAction,
}

impl CompositorRequest for PopupServerRequest {}

/// Queue of pending popup-management requests to be applied by platform lifecycle systems.
pub type PendingPopupServerRequests = crate::kinds::CompositorRequestQueue<PopupServerRequest>;

/// Public status snapshot for the compositor's Wayland protocol server socket.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolServerState {
    pub socket_name: Option<String>,
    pub runtime_dir: Option<String>,
    pub startup_error: Option<String>,
    pub last_accept_error: Option<String>,
    pub last_dispatch_error: Option<String>,
}

/// Public status snapshot for the compositor's XWayland server integration.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct XWaylandServerState {
    pub enabled: bool,
    pub ready: bool,
    pub display_number: Option<u32>,
    pub display_name: Option<String>,
    pub startup_error: Option<String>,
    pub last_error: Option<String>,
}
