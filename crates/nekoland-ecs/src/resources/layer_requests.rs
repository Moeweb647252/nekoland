use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::{LayerAnchor, LayerLevel, LayerMargins};

use super::pending_events::SurfaceExtent;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerSurfaceCreateSpec {
    pub namespace: String,
    pub output_name: Option<String>,
    pub layer: LayerLevel,
    pub anchor: LayerAnchor,
    pub desired_width: u32,
    pub desired_height: u32,
    pub exclusive_zone: i32,
    pub margins: LayerMargins,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerLifecycleAction {
    Created {
        spec: LayerSurfaceCreateSpec,
    },
    Committed {
        size: Option<SurfaceExtent>,
        anchor: LayerAnchor,
        desired_width: u32,
        desired_height: u32,
        exclusive_zone: i32,
        margins: LayerMargins,
    },
    Destroyed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerLifecycleRequest {
    pub surface_id: u64,
    pub action: LayerLifecycleAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingLayerRequests {
    pub items: Vec<LayerLifecycleRequest>,
}
