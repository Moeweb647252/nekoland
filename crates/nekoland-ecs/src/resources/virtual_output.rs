use std::collections::VecDeque;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Element kinds that can appear in a virtual-output frame capture.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum VirtualOutputElementKind {
    Window,
    Popup,
    Layer,
    Quad,
    Backdrop,
    Compositor,
    Cursor,
    #[default]
    Unknown,
}

/// One renderable item captured in a virtual-output frame snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VirtualOutputElement {
    pub surface_id: u64,
    pub kind: VirtualOutputElementKind,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z_index: i32,
    pub opacity: f32,
}

/// Captured virtual frame used by tests and tooling instead of a real renderer.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VirtualOutputFrame {
    pub output_name: String,
    pub frame: u64,
    pub uptime_millis: u64,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub background_color: String,
    /// Canonical present-path elements captured from the active `RenderPlan` consumer path.
    pub elements: Vec<VirtualOutputElement>,
}

/// Ring buffer of recent virtual-output frames.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct VirtualOutputCaptureState {
    pub frame_limit: usize,
    pub frames: VecDeque<VirtualOutputFrame>,
}

impl Default for VirtualOutputCaptureState {
    fn default() -> Self {
        Self { frame_limit: 4, frames: VecDeque::new() }
    }
}

impl VirtualOutputCaptureState {
    /// Appends a new captured frame and truncates the history to the configured frame limit.
    pub fn push_frame(&mut self, frame: VirtualOutputFrame) {
        self.frames.push_back(frame);
        while self.frames.len() > self.frame_limit.max(1) {
            self.frames.pop_front();
        }
    }
}
