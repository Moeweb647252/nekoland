//! Tiling-layout commands accepted over IPC.

#![allow(missing_docs)]

pub use nekoland_ecs::resources::{
    HorizontalDirection, TilingPanDirection, VerticalDirection,
};
use serde::{Deserialize, Serialize};

/// Tiling-management commands accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TilingCommand {
    FocusColumn { direction: HorizontalDirection },
    FocusWindow { direction: VerticalDirection },
    MoveColumn { direction: HorizontalDirection },
    MoveWindow { direction: VerticalDirection },
    ConsumeIntoColumn { direction: HorizontalDirection },
    ExpelFromColumn { direction: HorizontalDirection },
    PanViewport { direction: TilingPanDirection },
}
