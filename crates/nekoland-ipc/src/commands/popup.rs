use serde::{Deserialize, Serialize};

/// Popup-management commands accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PopupCommand {
    Dismiss { surface_id: u64 },
}
