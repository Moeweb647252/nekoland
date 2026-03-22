use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::{OutputId, SurfaceGeometry};

/// Minimal render/present-facing surface classification shared across platform boundaries.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderSurfaceRole {
    Window,
    Popup,
    Layer,
    #[default]
    Unknown,
}

/// Stable present-time surface snapshot safe to share outside backend internals.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderSurfaceSnapshot {
    pub geometry: SurfaceGeometry,
    pub role: RenderSurfaceRole,
    pub target_output: Option<OutputId>,
}

/// Latest normalized present-time surface snapshots keyed by compositor surface id.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PresentSurfaceSnapshotState {
    pub surfaces: BTreeMap<u64, RenderSurfaceSnapshot>,
}
