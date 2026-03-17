use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Generic output-local rectangle used by render-plan instances.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<&crate::components::SurfaceGeometry> for RenderRect {
    fn from(value: &crate::components::SurfaceGeometry) -> Self {
        Self { x: value.x, y: value.y, width: value.width, height: value.height }
    }
}

/// Generic render-scene layer. The initial abstraction keeps the enum intentionally small.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderSceneRole {
    #[default]
    Desktop,
    Overlay,
    Chrome,
    Cursor,
}

/// One surface instance for a specific output in the current frame.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SurfaceRenderItem {
    pub surface_id: u64,
    pub rect: RenderRect,
    pub opacity: f32,
    pub z_index: i32,
    pub clip_rect: Option<RenderRect>,
    pub scene_role: RenderSceneRole,
}

/// One generic render-plan item. The first implementation only uses surface items.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RenderPlanItem {
    Surface(SurfaceRenderItem),
}

impl RenderPlanItem {
    pub fn z_index(&self) -> i32 {
        match self {
            Self::Surface(item) => item.z_index,
        }
    }
}

/// Ordered render items for one output.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputRenderPlan {
    pub items: Vec<RenderPlanItem>,
}

impl OutputRenderPlan {
    pub fn push(&mut self, item: RenderPlanItem) {
        self.items.push(item);
    }

    pub fn sort_by_z_index(&mut self) {
        self.items.sort_by_key(RenderPlanItem::z_index);
    }
}

/// Output-scoped render plan built in parallel with the legacy render-list path.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderPlan {
    pub outputs: BTreeMap<OutputId, OutputRenderPlan>,
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;
    use crate::resources::{
        OutputRenderPlan, RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole,
        SurfaceRenderItem,
    };

    #[test]
    fn output_render_plan_sorts_by_z_index() {
        let mut plan = OutputRenderPlan {
            items: vec![
                RenderPlanItem::Surface(SurfaceRenderItem {
                    surface_id: 2,
                    rect: RenderRect::default(),
                    opacity: 1.0,
                    z_index: 4,
                    clip_rect: None,
                    scene_role: RenderSceneRole::Desktop,
                }),
                RenderPlanItem::Surface(SurfaceRenderItem {
                    surface_id: 1,
                    rect: RenderRect::default(),
                    opacity: 1.0,
                    z_index: 1,
                    clip_rect: None,
                    scene_role: RenderSceneRole::Desktop,
                }),
            ],
        };

        plan.sort_by_z_index();

        let surface_ids = plan
            .items
            .into_iter()
            .map(|item| match item {
                RenderPlanItem::Surface(item) => item.surface_id,
            })
            .collect::<Vec<_>>();
        assert_eq!(surface_ids, vec![1, 2]);
    }

    #[test]
    fn render_plan_keys_output_plans_by_output_id() {
        let plan = RenderPlan {
            outputs: std::collections::BTreeMap::from([
                (OutputId(3), OutputRenderPlan::default()),
                (OutputId(1), OutputRenderPlan::default()),
            ]),
        };

        let ids = plan.outputs.keys().copied().collect::<Vec<_>>();
        assert_eq!(ids, vec![OutputId(1), OutputId(3)]);
    }
}
