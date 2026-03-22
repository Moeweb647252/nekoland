use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Normalized frame-local output geometry snapshot for shell/input consumers.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputSnapshotState {
    pub outputs: Vec<OutputGeometrySnapshot>,
}

/// Stable output-local geometry and presentation metadata exported without runtime handles.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputGeometrySnapshot {
    pub output_id: OutputId,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub refresh_millihz: u32,
}

impl OutputGeometrySnapshot {
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let left = self.x as f64;
        let top = self.y as f64;
        let right = left + f64::from(self.width.max(1));
        let bottom = top + f64::from(self.height.max(1));
        x >= left && x < right && y >= top && y < bottom
    }
}
