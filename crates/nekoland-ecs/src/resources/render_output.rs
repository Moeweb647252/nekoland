//! Final present-target summaries exported by the render pipeline.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{RenderPassId, RenderTargetId};

/// Explicit per-output present target exported by the render subapp.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderFinalOutputPlan {
    pub outputs: BTreeMap<OutputId, OutputFinalTargetPlan>,
}

/// Stable summary of the pass/target pair that produces the final presentable output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputFinalTargetPlan {
    pub present_pass_id: RenderPassId,
    pub present_target_id: RenderTargetId,
    pub content_target_id: RenderTargetId,
}
