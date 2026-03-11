use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PopupCommand {
    Dismiss { surface_id: u64 },
}
