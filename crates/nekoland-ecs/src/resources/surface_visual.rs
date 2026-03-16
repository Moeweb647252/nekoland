use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// User-facing visual state derived from animation/effect systems for one surface.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SurfaceVisualState {
    pub opacity: f32,
}

impl Default for SurfaceVisualState {
    fn default() -> Self {
        Self { opacity: 1.0 }
    }
}

/// Per-surface visual snapshot consumed by compositor rendering without exposing animation internals.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SurfaceVisualSnapshot {
    pub surfaces: BTreeMap<u64, SurfaceVisualState>,
}
