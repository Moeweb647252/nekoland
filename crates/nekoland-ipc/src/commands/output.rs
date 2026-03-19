use serde::{Deserialize, Serialize};

/// One output-overlay rectangle encoded through IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputOverlayRectCommand {
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}

/// One output-overlay color encoded through IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputOverlayColorCommand {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

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
    OverlaySet {
        output: String,
        overlay_id: String,
        rect: OutputOverlayRectCommand,
        color: OutputOverlayColorCommand,
        #[serde(default)]
        opacity: Option<f32>,
        #[serde(default)]
        z_index: Option<i32>,
        #[serde(default)]
        clip_rect: Option<OutputOverlayRectCommand>,
    },
    OverlayRemove {
        output: String,
        overlay_id: String,
    },
    OverlayClear {
        output: String,
    },
}
