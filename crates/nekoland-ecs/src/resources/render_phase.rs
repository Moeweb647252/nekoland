//! Intermediate render-phase plans built before graph compilation.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{
    MaterialParamsId, ProcessRect, RenderItemId, RenderMaterialId, RenderSceneRole,
    ScreenshotRequestId,
};

/// Generic render-phase plan built before graph compilation.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderPhasePlan {
    pub outputs: BTreeMap<OutputId, OutputPhasePlan>,
}

/// Output-local phase lists consumed by the render-graph compiler.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPhasePlan {
    pub scene_passes: Vec<ScenePhaseItem>,
    pub post_process_passes: Vec<PostProcessPhaseItem>,
    pub readback: Option<ReadbackPhaseItem>,
}

/// One scene phase item carrying ordered render-plan item ids for a single scene role.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePhaseItem {
    pub scene_role: RenderSceneRole,
    pub item_ids: Vec<RenderItemId>,
}

/// One post-process phase item carrying one typed material request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostProcessPhaseItem {
    pub scene_role: RenderSceneRole,
    pub material_id: RenderMaterialId,
    pub params_id: Option<MaterialParamsId>,
    pub process_regions: Vec<ProcessRect>,
}

/// One readback phase item carrying all screenshot request ids for an output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadbackPhaseItem {
    pub request_ids: Vec<ScreenshotRequestId>,
}
