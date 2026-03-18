use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::{MaterialParamsId, RenderMaterialId, RenderRect};

/// Stable backend-readable material pipeline key.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct RenderMaterialPipelineKey(pub String);

/// Backend-readable descriptor for one material referenced by the render graph.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderMaterialDescriptor {
    pub debug_name: String,
    pub pipeline_key: RenderMaterialPipelineKey,
}

/// Generic material parameter payload value.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum RenderMaterialParamValue {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    Rect(RenderRect),
    Bool(bool),
    Uint(u32),
}

/// Backend-readable parameter block for one material pass.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderMaterialParamBlock {
    pub values: BTreeMap<String, RenderMaterialParamValue>,
}

impl RenderMaterialParamBlock {
    pub fn float(&self, key: &str) -> Option<f32> {
        match self.values.get(key) {
            Some(RenderMaterialParamValue::Float(value)) => Some(*value),
            _ => None,
        }
    }

    pub fn rect(&self, key: &str) -> Option<RenderRect> {
        match self.values.get(key) {
            Some(RenderMaterialParamValue::Rect(value)) => Some(*value),
            _ => None,
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
    use crate::resources::{RenderMaterialDescriptor, RenderMaterialFrameState, RenderMaterialId};

    use super::{MaterialParamsId, RenderMaterialParamBlock, RenderMaterialPipelineKey};

    #[test]
    fn frame_state_returns_registered_descriptors_and_params() {
        let state = RenderMaterialFrameState {
            descriptors: std::collections::BTreeMap::from([(
                RenderMaterialId(3),
                RenderMaterialDescriptor {
                    debug_name: "backdrop_blur".to_owned(),
                    pipeline_key: RenderMaterialPipelineKey("backdrop_blur".to_owned()),
                },
            )]),
            params: std::collections::BTreeMap::from([(
                MaterialParamsId(5),
                RenderMaterialParamBlock::default(),
            )]),
        };

        assert_eq!(
            state.descriptor(RenderMaterialId(3)).map(|descriptor| descriptor.debug_name.as_str()),
            Some("backdrop_blur")
        );
        assert_eq!(state.params(MaterialParamsId(5)), Some(&RenderMaterialParamBlock::default()));
    }
}
