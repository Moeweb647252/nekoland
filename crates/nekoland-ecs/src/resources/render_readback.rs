//! Screenshot/readback execution plans derived from the render graph.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{RenderTargetId, ScreenshotRequestId};

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderReadbackPlan {
    pub outputs: BTreeMap<OutputId, OutputReadbackPlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputReadbackPlan {
    pub source_target: RenderTargetId,
    pub request_ids: Vec<ScreenshotRequestId>,
}
