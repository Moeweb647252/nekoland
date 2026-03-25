//! Output identity, placement, viewport, and property components.
#![allow(missing_docs)]

use std::sync::atomic::{AtomicU64, Ordering};

use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

use crate::components::{WorkspaceCoord, WorkspaceId};

static NEXT_OUTPUT_ID: AtomicU64 = AtomicU64::new(1);

/// Runtime-stable identity for one output entity.
#[derive(
    Component, Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct OutputId(pub u64);

impl OutputId {
    pub fn fresh() -> Self {
        Self(NEXT_OUTPUT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for OutputId {
    fn default() -> Self {
        Self::fresh()
    }
}

/// Broad output families exposed by the compositor.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputKind {
    Physical,
    Nested,
    #[default]
    Virtual,
}

/// Stable identity metadata for an output entity.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputViewport {
    pub origin_x: WorkspaceCoord,
    pub origin_y: WorkspaceCoord,
}

impl OutputViewport {
    pub fn move_to(&mut self, x: WorkspaceCoord, y: WorkspaceCoord) {
        self.origin_x = x;
        self.origin_y = y;
    }

    pub fn pan_by(&mut self, delta_x: WorkspaceCoord, delta_y: WorkspaceCoord) {
        self.origin_x = self.origin_x.saturating_add(delta_x);
        self.origin_y = self.origin_y.saturating_add(delta_y);
    }
}

/// Placement of an output in compositor-global pointer space.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPlacement {
    pub x: i32,
    pub y: i32,
}

/// Per-output work area after layer-shell exclusive zones have been applied.
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputWorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for OutputWorkArea {
    fn default() -> Self {
        Self { x: 0, y: 0, width: 1280, height: 720 }
    }
}

/// Output-scoped workspace routing state.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputCurrentWorkspace {
    pub workspace: WorkspaceId,
}

/// Stable identity metadata for an output entity.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[require(OutputId, OutputProperties, OutputViewport, OutputPlacement, OutputWorkArea)]
pub struct OutputDevice {
    pub name: String,
    pub kind: OutputKind,
    pub make: String,
    pub model: String,
}

/// Mutable output mode properties used by layout, rendering, and IPC snapshots.
#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputProperties {
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
    pub scale: u32,
}
