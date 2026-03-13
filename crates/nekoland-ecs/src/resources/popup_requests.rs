use crate::kinds::CompositorRequestQueue;
use serde::{Deserialize, Serialize};

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

/// Queue of pending popup-management requests to be applied by popup lifecycle systems.
pub type PendingPopupServerRequests = CompositorRequestQueue<PopupServerRequest>;
