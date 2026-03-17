use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// One damaged rectangle in output-local coordinates.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DamageRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Coarse damage mode for the current frame.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DamageState {
    pub full_redraw: bool,
}

/// Per-output damage rectangles derived for the current frame.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputDamageRegions {
    pub regions: BTreeMap<OutputId, Vec<DamageRect>>,
}
