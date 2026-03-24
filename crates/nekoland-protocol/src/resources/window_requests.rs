use super::pending_events::SurfaceExtent;
use super::x11_requests::X11WindowGeometry;
use nekoland_ecs::kinds::CompositorRequestQueue;
use serde::{Deserialize, Serialize};

/// Internal protocol bridge requests for windows.
///
/// New user-facing control flows should go through `PendingWindowControls` and `WindowOps`.
/// This queue remains only for the final protocol bridge after shell-side reconciliation has
/// already decided that a close or presentation-state sync should happen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowServerAction {
    Close,
    SyncXdgToplevelState {
        size: Option<SurfaceExtent>,
        fullscreen: bool,
        maximized: bool,
        resizing: bool,
    },
    SyncX11WindowPresentation { geometry: X11WindowGeometry, fullscreen: bool, maximized: bool },
}

/// One low-level window request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowServerRequest {
    pub surface_id: u64,
    pub action: WindowServerAction,
}

/// Queue of pending protocol-bridge window requests.
pub type PendingWindowServerRequests = CompositorRequestQueue<WindowServerRequest>;
