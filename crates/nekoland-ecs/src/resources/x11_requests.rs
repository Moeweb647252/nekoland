use serde::{Deserialize, Serialize};

use crate::kinds::ProtocolEventQueue;
use crate::resources::{ResizeEdges, SurfaceExtent};

/// Geometry reported for one X11 window.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct X11WindowGeometry {
    /// Left coordinate in compositor logical space.
    pub x: i32,
    /// Top coordinate in compositor logical space.
    pub y: i32,
    /// Window width in logical pixels.
    pub width: u32,
    /// Window height in logical pixels.
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
    /// Initial map notification carrying basic window metadata.
    Mapped {
        window_id: u32,
        override_redirect: bool,
        title: String,
        app_id: String,
        geometry: X11WindowGeometry,
    },
    /// Geometry or metadata refresh for an already-mapped X11 window.
    Reconfigured {
        title: String,
        app_id: String,
        geometry: X11WindowGeometry,
    },
    Maximize,
    UnMaximize,
    Fullscreen,
    UnFullscreen,
    Minimize,
    UnMinimize,
    /// Begin an interactive move operation initiated by the X11 client.
    InteractiveMove {
        button: u32,
    },
    /// Begin an interactive resize operation initiated by the X11 client.
    InteractiveResize {
        button: u32,
        edges: ResizeEdges,
    },
    /// Surface became unmapped but not yet destroyed.
    Unmapped,
    /// Final teardown notification.
    Destroyed,
}

/// One X11 lifecycle request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct X11LifecycleRequest {
    /// Protocol/XWayland surface id associated with the X11 window.
    pub surface_id: u64,
    /// Lifecycle action to apply to the X11-backed entity.
    pub action: X11LifecycleAction,
}

/// Queue of pending X11 lifecycle requests.
pub type PendingX11Requests = ProtocolEventQueue<X11LifecycleRequest>;
