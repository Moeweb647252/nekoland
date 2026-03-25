//! Window-management commands accepted over IPC.

#![allow(missing_docs)]

pub use nekoland_ecs::resources::SplitAxis;
use serde::{Deserialize, Serialize};

/// Window-management commands accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowCommand {
    Focus { surface_id: u64 },
    Close { surface_id: u64 },
    Move { surface_id: u64, x: i64, y: i64 },
    Resize { surface_id: u64, width: u32, height: u32 },
    Split { surface_id: u64, axis: SplitAxis },
    Background { surface_id: u64, output: String },
    ClearBackground { surface_id: u64 },
}
