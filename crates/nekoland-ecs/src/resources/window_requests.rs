use crate::kinds::CompositorRequestQueue;
use serde::{Deserialize, Serialize};

/// Internal protocol bridge requests for windows.
///
/// New user-facing control flows should go through `PendingWindowControls` and `WindowOps`.
/// This queue remains only for the final protocol close bridge after shell-side reconciliation has
/// already decided that a close should happen.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowServerAction {
    Close,
}

/// One low-level window request targeted at a surface id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowServerRequest {
    pub surface_id: u64,
    pub action: WindowServerAction,
}

/// Queue of pending protocol-bridge window requests.
pub type PendingWindowServerRequests = CompositorRequestQueue<WindowServerRequest>;
