use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Generic output-local rectangle used by render-plan instances.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
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

impl RenderRect {
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self
            .x
            .saturating_add(self.width.min(i32::MAX as u32) as i32)
            .min(other.x.saturating_add(other.width.min(i32::MAX as u32) as i32));
        let bottom = self
            .y
            .saturating_add(self.height.min(i32::MAX as u32) as i32)
            .min(other.y.saturating_add(other.height.min(i32::MAX as u32) as i32));

        (right > left && bottom > top).then_some(Self {
            x: left,
            y: top,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }
}

/// Generic render-scene layer. The initial abstraction keeps the enum intentionally small.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderSceneRole {
    #[default]
    Desktop,
    Overlay,
    Compositor,
    Cursor,
}

/// Common per-instance metadata shared by every render-plan item.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct RenderItemInstance {
    pub rect: RenderRect,
    pub opacity: f32,
    pub clip_rect: Option<RenderRect>,
    pub z_index: i32,
    pub scene_role: RenderSceneRole,
}

impl RenderItemInstance {
    pub fn visible_rect(self) -> Option<RenderRect> {
        match self.clip_rect {
            Some(clip_rect) => self.rect.intersection(clip_rect),
            None if self.rect.is_empty() => None,
            None => Some(self.rect),
        }
    }
}

/// One surface instance for a specific output in the current frame.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SurfaceRenderItem {
    pub surface_id: u64,
    pub instance: RenderItemInstance,
}

/// One compositor-generated colored rectangle for an output-local scene.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// One solid-color rectangle in the current output-local scene.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SolidRectRenderItem {
    pub color: RenderColor,
    pub instance: RenderItemInstance,
}

/// One sampled output-local backdrop region reserved for future post-process work.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct BackdropRenderItem {
    pub instance: RenderItemInstance,
}

/// One generic render-plan item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RenderPlanItem {
    Surface(SurfaceRenderItem),
    SolidRect(SolidRectRenderItem),
    Backdrop(BackdropRenderItem),
}

impl RenderPlanItem {
    pub fn instance(&self) -> &RenderItemInstance {
        match self {
            Self::Surface(item) => &item.instance,
            Self::SolidRect(item) => &item.instance,
            Self::Backdrop(item) => &item.instance,
        }
    }

    pub fn z_index(&self) -> i32 {
        self.instance().z_index
    }

    pub fn surface_id(&self) -> Option<u64> {
        match self {
            Self::Surface(item) => Some(item.surface_id),
            Self::SolidRect(_) | Self::Backdrop(_) => None,
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
        BackdropRenderItem, OutputRenderPlan, RenderColor, RenderItemInstance, RenderPlan,
        RenderPlanItem, RenderRect, RenderSceneRole, SolidRectRenderItem, SurfaceRenderItem,
    };

    #[test]
    fn output_render_plan_sorts_by_z_index() {
        let mut plan = OutputRenderPlan {
            items: vec![
                RenderPlanItem::Surface(SurfaceRenderItem {
                    surface_id: 2,
                    instance: RenderItemInstance {
                        rect: RenderRect::default(),
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 4,
                        scene_role: RenderSceneRole::Desktop,
                    },
                }),
                RenderPlanItem::SolidRect(SolidRectRenderItem {
                    color: RenderColor { r: 0, g: 0, b: 0, a: 255 },
                    instance: RenderItemInstance {
                        rect: RenderRect::default(),
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 1,
                        scene_role: RenderSceneRole::Compositor,
                    },
                }),
            ],
        };

        plan.sort_by_z_index();

        let item_kinds = plan
            .items
            .into_iter()
            .map(|item| match item {
                RenderPlanItem::Surface(item) => format!("surface-{}", item.surface_id),
                RenderPlanItem::SolidRect(_) => "solid-rect".to_owned(),
                RenderPlanItem::Backdrop(_) => "backdrop".to_owned(),
            })
            .collect::<Vec<_>>();
        assert_eq!(item_kinds, vec!["solid-rect", "surface-2"]);
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

    #[test]
    fn item_instance_visible_rect_respects_clip_rect() {
        let item = RenderItemInstance {
            rect: RenderRect { x: 10, y: 10, width: 100, height: 80 },
            opacity: 1.0,
            clip_rect: Some(RenderRect { x: 40, y: 0, width: 100, height: 30 }),
            z_index: 0,
            scene_role: RenderSceneRole::Overlay,
        };

        assert_eq!(item.visible_rect(), Some(RenderRect { x: 40, y: 10, width: 70, height: 20 }),);
    }

    #[test]
    fn render_plan_item_surface_id_only_exists_for_surface_items() {
        let surface = RenderPlanItem::Surface(SurfaceRenderItem {
            surface_id: 7,
            instance: RenderItemInstance {
                rect: RenderRect::default(),
                opacity: 1.0,
                clip_rect: None,
                z_index: 0,
                scene_role: RenderSceneRole::Desktop,
            },
        });
        let backdrop = RenderPlanItem::Backdrop(BackdropRenderItem {
            instance: RenderItemInstance {
                rect: RenderRect::default(),
                opacity: 1.0,
                clip_rect: None,
                z_index: 1,
                scene_role: RenderSceneRole::Overlay,
            },
        });

        assert_eq!(surface.surface_id(), Some(7));
        assert_eq!(backdrop.surface_id(), None);
    }
}
