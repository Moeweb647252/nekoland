use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Output-local cursor snapshot produced during the render phase and consumed by present backends.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CursorRenderState {
    pub visible: bool,
    pub output_id: Option<OutputId>,
    pub x: f64,
    pub y: f64,
}
