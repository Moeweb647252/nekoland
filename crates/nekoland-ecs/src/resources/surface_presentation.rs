//! Shell-owned surface presentation snapshots used by render and backend extraction.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::{OutputId, SurfaceGeometry};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurfacePresentationRole {
    #[default]
    Window,
    Popup,
    Layer,
    OutputBackground,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfacePresentationState {
    pub visible: bool,
    pub target_output: Option<OutputId>,
    pub geometry: SurfaceGeometry,
    pub input_enabled: bool,
    pub damage_enabled: bool,
    pub role: SurfacePresentationRole,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfacePresentationSnapshot {
    pub surfaces: BTreeMap<u64, SurfacePresentationState>,
}
