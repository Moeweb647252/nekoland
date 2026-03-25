//! Fully compiled per-output frame payloads exported from the render sub-app.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{
    DamageRect, OutputDamageRegions, OutputExecutionPlan, OutputFinalTargetPlan,
    OutputPreparedGpuResources, OutputPreparedSceneResources, OutputProcessPlan,
    OutputReadbackPlan, OutputRenderPlan, OutputTargetAllocationPlan, PreparedGpuResources,
    PreparedSceneResources, RenderFinalOutputPlan, RenderMaterialFrameState, RenderPassGraph,
    RenderPlan, RenderProcessPlan, RenderReadbackPlan, RenderTargetAllocationPlan,
    SurfaceTextureBridgePlan,
};

/// Stable per-output compiled frame exported to present backends.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CompiledOutputFrame {
    pub render_plan: OutputRenderPlan,
    pub prepared_scene: OutputPreparedSceneResources,
    pub execution_plan: OutputExecutionPlan,
    pub process_plan: OutputProcessPlan,
    pub final_output: Option<OutputFinalTargetPlan>,
    pub readback: Option<OutputReadbackPlan>,
    pub target_allocation: Option<OutputTargetAllocationPlan>,
    pub gpu_prep: Option<OutputPreparedGpuResources>,
    pub damage_regions: Vec<DamageRect>,
}

/// Aggregated present-time output data exported by the render pipeline.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CompiledOutputFrames {
    pub outputs: BTreeMap<OutputId, CompiledOutputFrame>,
    pub output_damage_regions: OutputDamageRegions,
    pub prepared_scene: PreparedSceneResources,
    pub materials: RenderMaterialFrameState,
    pub render_graph: RenderPassGraph,
    pub render_plan: RenderPlan,
    pub process_plan: RenderProcessPlan,
    pub final_output_plan: RenderFinalOutputPlan,
    pub readback_plan: RenderReadbackPlan,
    pub render_target_allocation: RenderTargetAllocationPlan,
    pub surface_texture_bridge: SurfaceTextureBridgePlan,
    pub prepared_gpu: PreparedGpuResources,
}

impl CompiledOutputFrames {
    /// Returns the compiled frame payload for one output.
    pub fn output(&self, output_id: OutputId) -> Option<&CompiledOutputFrame> {
        self.outputs.get(&output_id)
    }
}
