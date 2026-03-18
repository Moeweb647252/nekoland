use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::RenderRect;

/// User-facing visual state derived from animation/effect systems for one surface.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SurfaceVisualState {
    pub opacity: f32,
    pub rect_override: Option<RenderRect>,
    pub clip_rect_override: Option<RenderRect>,
}

impl Default for SurfaceVisualState {
    fn default() -> Self {
        Self { opacity: 1.0, rect_override: None, clip_rect_override: None }
    }
}

/// Per-surface visual snapshot consumed by compositor rendering without exposing animation internals.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SurfaceVisualSnapshot {
    pub surfaces: BTreeMap<u64, SurfaceVisualState>,
}
