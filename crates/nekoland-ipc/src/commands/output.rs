//! Output-management commands accepted over IPC.

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// Mutable output-management commands accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutputCommand {
    Configure {
        output: String,
        mode: String,
        #[serde(default)]
        scale: Option<u32>,
    },
    Enable {
        output: String,
    },
    Disable {
        output: String,
    },
    ViewportMove {
        output: String,
        x: i64,
        y: i64,
    },
    ViewportPan {
        output: String,
        dx: i64,
        dy: i64,
    },
    CenterViewportOnWindow {
        output: String,
        surface_id: u64,
    },
}
