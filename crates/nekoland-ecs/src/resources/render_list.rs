use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// One composed render entry after shell/layout systems have decided visibility and ordering.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderElement {
    pub surface_id: u64,
    pub z_index: i32,
    pub opacity: f32,
}

/// Ordered render list consumed by render and virtual-output systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderList {
    pub elements: Vec<RenderElement>,
}
