use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::{MaterialParamsId, RenderMaterialId};

/// Stable material family identifier used by pipeline specialization.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum RenderMaterialKind {
    #[default]
    Generic,
    BackdropBlur,
    Blur,
    Shadow,
    RoundedCorners,
}

/// Stable pipeline stage classification used by render/executor specialization.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum RenderPipelineStage {
    #[default]
    PostProcess,
    Scene,
    Composite,
}

/// Stable backend-readable material pipeline key.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenderMaterialPipelineKey {
    pub material: RenderMaterialKind,
    pub stage: RenderPipelineStage,
}

impl RenderMaterialPipelineKey {
    pub const fn new(material: RenderMaterialKind, stage: RenderPipelineStage) -> Self {
        Self { material, stage }
    }

    pub const fn post_process(material: RenderMaterialKind) -> Self {
        Self::new(material, RenderPipelineStage::PostProcess)
    }

    pub fn debug_name(&self) -> &'static str {
        match self.material {
            RenderMaterialKind::Generic => "generic",
            RenderMaterialKind::BackdropBlur => "backdrop_blur",
            RenderMaterialKind::Blur => "blur",
            RenderMaterialKind::Shadow => "shadow",
            RenderMaterialKind::RoundedCorners => "rounded_corners",
        }
    }
}

/// Stable shader entry family selected for one material.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderMaterialShaderSource {
    #[default]
    Generic,
    BackdropBlur,
    Blur,
    Shadow,
    RoundedCorners,
}

/// Stable bind-group layout family required by one material.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderBindGroupLayoutKey {
    #[default]
    Generic,
    BlurUniforms,
    ShadowUniforms,
    RoundedCornerUniforms,
}

/// Stable queue classification used when projecting material requests into passes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderMaterialQueueKind {
    #[default]
    PostProcess,
    BackdropPostProcess,
    Mask,
}

/// Backend-readable descriptor for one material referenced by the render graph.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderMaterialDescriptor {
    pub debug_name: String,
    pub pipeline_key: RenderMaterialPipelineKey,
    pub shader_source: RenderMaterialShaderSource,
    pub bind_group_layout: RenderBindGroupLayoutKey,
    pub queue_kind: RenderMaterialQueueKind,
}

/// Typed blur-like parameter block shared by blur and backdrop-blur materials.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BlurMaterialParams {
    pub radius: f32,
}

/// Typed shadow parameter block.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShadowMaterialParams {
    pub spread: f32,
    pub offset: [f32; 2],
    pub color: [f32; 4],
}

/// Typed rounded-corner parameter block.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RoundedCornerMaterialParams {
    pub radius: f32,
}

/// Backend-readable typed parameter block for one material pass.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum RenderMaterialParamBlock {
    #[default]
    Empty,
    Blur(BlurMaterialParams),
    Shadow(ShadowMaterialParams),
    RoundedCorners(RoundedCornerMaterialParams),
}

impl RenderMaterialParamBlock {
    pub fn blur(radius: f32) -> Self {
        Self::Blur(BlurMaterialParams { radius })
    }

    pub fn shadow(spread: f32, offset_x: f32, offset_y: f32, color: [f32; 4]) -> Self {
        Self::Shadow(ShadowMaterialParams { spread, offset: [offset_x, offset_y], color })
    }

    pub fn rounded_corners(radius: f32) -> Self {
        Self::RoundedCorners(RoundedCornerMaterialParams { radius })
    }

    pub fn radius(&self) -> Option<f32> {
        match self {
            Self::Blur(params) => Some(params.radius),
            Self::RoundedCorners(params) => Some(params.radius),
            Self::Empty | Self::Shadow(_) => None,
        }
    }
}

/// Backend-readable material frame state projected from render-local material requests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderMaterialFrameState {
    pub descriptors: BTreeMap<RenderMaterialId, RenderMaterialDescriptor>,
    pub params: BTreeMap<MaterialParamsId, RenderMaterialParamBlock>,
}

impl RenderMaterialFrameState {
    pub fn descriptor(&self, material_id: RenderMaterialId) -> Option<&RenderMaterialDescriptor> {
        self.descriptors.get(&material_id)
    }

    pub fn params(&self, params_id: MaterialParamsId) -> Option<&RenderMaterialParamBlock> {
        self.params.get(&params_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::resources::{
        BlurMaterialParams, RenderBindGroupLayoutKey, RenderMaterialDescriptor,
        RenderMaterialFrameState, RenderMaterialId, RenderMaterialKind, RenderMaterialQueueKind,
        RenderMaterialShaderSource, RenderPipelineStage, RoundedCornerMaterialParams,
        ShadowMaterialParams,
    };

    use super::{MaterialParamsId, RenderMaterialParamBlock, RenderMaterialPipelineKey};

    #[test]
    fn frame_state_returns_registered_descriptors_and_params() {
        let state = RenderMaterialFrameState {
            descriptors: std::collections::BTreeMap::from([(
                RenderMaterialId(3),
                RenderMaterialDescriptor {
                    debug_name: "backdrop_blur".to_owned(),
                    pipeline_key: RenderMaterialPipelineKey::post_process(
                        RenderMaterialKind::BackdropBlur,
                    ),
                    shader_source: RenderMaterialShaderSource::BackdropBlur,
                    bind_group_layout: RenderBindGroupLayoutKey::BlurUniforms,
                    queue_kind: RenderMaterialQueueKind::BackdropPostProcess,
                },
            )]),
            params: std::collections::BTreeMap::from([
                (MaterialParamsId(5), RenderMaterialParamBlock::blur(12.0)),
                (
                    MaterialParamsId(6),
                    RenderMaterialParamBlock::Shadow(ShadowMaterialParams {
                        spread: 3.0,
                        offset: [1.0, 2.0],
                        color: [0.0, 0.0, 0.0, 0.25],
                    }),
                ),
                (
                    MaterialParamsId(7),
                    RenderMaterialParamBlock::RoundedCorners(RoundedCornerMaterialParams {
                        radius: 10.0,
                    }),
                ),
            ]),
        };

        assert_eq!(
            state.descriptor(RenderMaterialId(3)).map(|descriptor| descriptor.debug_name.as_str()),
            Some("backdrop_blur")
        );
        assert_eq!(
            state.params(MaterialParamsId(5)),
            Some(&RenderMaterialParamBlock::Blur(BlurMaterialParams { radius: 12.0 }))
        );
        assert_eq!(
            state.params(MaterialParamsId(5)).and_then(RenderMaterialParamBlock::radius),
            Some(12.0)
        );
        assert_eq!(
            state.descriptor(RenderMaterialId(3)).map(|descriptor| descriptor.pipeline_key.stage),
            Some(RenderPipelineStage::PostProcess)
        );
    }
}
