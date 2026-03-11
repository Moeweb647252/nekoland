use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderElement {
    pub surface_id: u64,
    pub z_index: i32,
    pub opacity: f32,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderList {
    pub elements: Vec<RenderElement>,
}
