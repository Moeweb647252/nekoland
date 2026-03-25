//! Post-process execution units compiled from the render graph.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{RenderMaterialPipelineKey, RenderPassId, RenderTargetId};

#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct ProcessUnitId(pub u64);

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ProcessShaderKey {
    #[default]
    Passthrough,
    BuiltinComposite,
    Material(RenderMaterialPipelineKey),
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<crate::resources::RenderRect> for ProcessRect {
    fn from(value: crate::resources::RenderRect) -> Self {
        Self { x: value.x, y: value.y, width: value.width, height: value.height }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ProcessUniformValue {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    Rect(ProcessRect),
    Bool(bool),
    Uint(u32),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProcessUniformBlock {
    pub values: BTreeMap<String, ProcessUniformValue>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessInputRef {
    pub target_id: RenderTargetId,
    pub sample_rect: Option<ProcessRect>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessTargetRef {
    pub target_id: RenderTargetId,
    pub output_rect: Option<ProcessRect>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProcessUnit {
    pub shader_key: ProcessShaderKey,
    pub input: ProcessInputRef,
    pub output: ProcessTargetRef,
    pub uniforms: ProcessUniformBlock,
    pub process_regions: Vec<ProcessRect>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputProcessPlan {
    pub units: BTreeMap<ProcessUnitId, ProcessUnit>,
    pub ordered_units: Vec<ProcessUnitId>,
    pub pass_units: BTreeMap<RenderPassId, Vec<ProcessUnitId>>,
}

impl OutputProcessPlan {
    /// Iterates process units associated with one render pass.
    pub fn units_for_pass(&self, pass_id: RenderPassId) -> impl Iterator<Item = &ProcessUnit> {
        self.pass_units
            .get(&pass_id)
            .into_iter()
            .flatten()
            .filter_map(|unit_id| self.units.get(unit_id))
    }
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderProcessPlan {
    pub outputs: BTreeMap<OutputId, OutputProcessPlan>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::components::OutputId;
    use crate::resources::{
        OutputProcessPlan, ProcessInputRef, ProcessRect, ProcessShaderKey, ProcessTargetRef,
        ProcessUniformBlock, ProcessUnit, ProcessUnitId, RenderPassId, RenderProcessPlan,
        RenderTargetId,
    };

    #[test]
    fn output_process_plan_returns_units_by_pass() {
        let unit = ProcessUnit {
            shader_key: ProcessShaderKey::BuiltinComposite,
            input: ProcessInputRef { target_id: RenderTargetId(1), sample_rect: None },
            output: ProcessTargetRef { target_id: RenderTargetId(2), output_rect: None },
            uniforms: ProcessUniformBlock::default(),
            process_regions: vec![ProcessRect { x: 1, y: 2, width: 3, height: 4 }],
        };
        let plan = OutputProcessPlan {
            units: BTreeMap::from([(ProcessUnitId(9), unit.clone())]),
            ordered_units: vec![ProcessUnitId(9)],
            pass_units: BTreeMap::from([(RenderPassId(3), vec![ProcessUnitId(9)])]),
        };

        assert_eq!(plan.units_for_pass(RenderPassId(3)).cloned().collect::<Vec<_>>(), vec![unit]);
    }

    #[test]
    fn render_process_plan_is_keyed_by_output_id() {
        let plan = RenderProcessPlan {
            outputs: BTreeMap::from([
                (OutputId(2), OutputProcessPlan::default()),
                (OutputId(1), OutputProcessPlan::default()),
            ]),
        };

        assert_eq!(
            plan.outputs.keys().copied().collect::<Vec<_>>(),
            vec![OutputId(1), OutputId(2)]
        );
    }
}
