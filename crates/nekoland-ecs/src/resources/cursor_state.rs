use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Output-local cursor scene snapshot produced during render preparation.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CursorSceneSnapshot {
    pub visible: bool,
    pub output_id: Option<OutputId>,
    pub x: f64,
    pub y: f64,
}

/// Pure ECS cursor-image snapshot synchronized from protocol state.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CursorImageSnapshot {
    Hidden,
    Named { icon_name: String },
    Surface { surface_id: u64, hotspot_x: i32, hotspot_y: i32, width: u32, height: u32 },
}

impl Default for CursorImageSnapshot {
    fn default() -> Self {
        Self::Named { icon_name: "default".to_owned() }
    }
}
