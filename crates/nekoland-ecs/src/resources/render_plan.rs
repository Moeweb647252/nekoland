use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Stable identity for one logical render source across frames.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct RenderSourceId(pub u64);

/// Stable identity for one output-local render item across frames.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct RenderItemId(pub u64);

/// Stable item identity linking one output-local instance back to its source.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderItemIdentity {
    pub source_id: RenderSourceId,
    pub item_id: RenderItemId,
}

impl RenderItemIdentity {
    pub const fn new(source_id: RenderSourceId, item_id: RenderItemId) -> Self {
        Self { source_id, item_id }
    }
}

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
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
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
    pub identity: RenderItemIdentity,
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

/// CPU-side raster image payload attached to one quad.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuadRasterImage {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub pixels_rgba: Vec<u8>,
}

/// Content payload carried by one quad render item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum QuadContent {
    SolidColor { color: RenderColor },
    RasterImage { image: QuadRasterImage },
}

/// One generic quad in the current output-local scene.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QuadRenderItem {
    pub identity: RenderItemIdentity,
    pub content: QuadContent,
    pub instance: RenderItemInstance,
}

/// One sampled output-local backdrop region reserved for future post-process work.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct BackdropRenderItem {
    pub identity: RenderItemIdentity,
    pub instance: RenderItemInstance,
}

/// Cursor-image source carried by one cursor render item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CursorRenderSource {
    Named { icon_name: String },
    Surface { surface_id: u64 },
}

/// One output-local cursor item in the current frame scene.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CursorRenderItem {
    pub identity: RenderItemIdentity,
    pub source: CursorRenderSource,
    pub instance: RenderItemInstance,
}

/// One generic render-plan item.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RenderPlanItem {
    Surface(SurfaceRenderItem),
    Quad(QuadRenderItem),
    Backdrop(BackdropRenderItem),
    Cursor(CursorRenderItem),
}

impl RenderPlanItem {
    pub fn identity(&self) -> RenderItemIdentity {
        match self {
            Self::Surface(item) => item.identity,
            Self::Quad(item) => item.identity,
            Self::Backdrop(item) => item.identity,
            Self::Cursor(item) => item.identity,
        }
    }

    pub fn item_id(&self) -> RenderItemId {
        self.identity().item_id
    }

    pub fn source_id(&self) -> RenderSourceId {
        self.identity().source_id
    }

    pub fn instance(&self) -> &RenderItemInstance {
        match self {
            Self::Surface(item) => &item.instance,
            Self::Quad(item) => &item.instance,
            Self::Backdrop(item) => &item.instance,
            Self::Cursor(item) => &item.instance,
        }
    }

    pub fn z_index(&self) -> i32 {
        self.instance().z_index
    }

    pub fn surface_id(&self) -> Option<u64> {
        match self {
            Self::Surface(item) => Some(item.surface_id),
            Self::Cursor(item) => match &item.source {
                CursorRenderSource::Surface { surface_id } => Some(*surface_id),
                CursorRenderSource::Named { .. } => None,
            },
            Self::Quad(_) | Self::Backdrop(_) => None,
        }
    }
}

/// Ordered render items for one output.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputRenderPlan {
    pub items: BTreeMap<RenderItemId, RenderPlanItem>,
    pub ordered_items: Vec<RenderItemId>,
}

impl OutputRenderPlan {
    pub fn from_items(items: impl IntoIterator<Item = RenderPlanItem>) -> Self {
        let mut plan = Self::default();
        for item in items {
            plan.insert(item);
        }
        plan.sort_by_z_index();
        plan
    }

    pub fn insert(&mut self, item: RenderPlanItem) {
        let item_id = item.item_id();
        let is_new = self.items.insert(item_id, item).is_none();
        if is_new {
            self.ordered_items.push(item_id);
        }
    }

    pub fn sort_by_z_index(&mut self) {
        self.ordered_items.sort_by_key(|item_id| {
            self.items.get(item_id).map(RenderPlanItem::z_index).unwrap_or(i32::MAX)
        });
    }

    pub fn ordered_item_ids(&self) -> &[RenderItemId] {
        &self.ordered_items
    }

    pub fn item(&self, item_id: RenderItemId) -> Option<&RenderPlanItem> {
        self.items.get(&item_id)
    }

    pub fn iter_ordered(&self) -> impl Iterator<Item = &RenderPlanItem> {
        self.ordered_items.iter().filter_map(|item_id| self.items.get(item_id))
    }
}

/// Output-scoped frame scene truth built from stable scene-source contributions.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderPlan {
    pub outputs: BTreeMap<OutputId, OutputRenderPlan>,
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;
    use crate::resources::{
        BackdropRenderItem, CursorRenderItem, CursorRenderSource, OutputRenderPlan, QuadContent,
        QuadRenderItem, RenderColor, RenderItemId, RenderItemIdentity, RenderItemInstance,
        RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole, RenderSourceId,
        SurfaceRenderItem,
    };

    #[test]
    fn output_render_plan_sorts_by_z_index() {
        let mut plan = OutputRenderPlan { ..Default::default() };
        plan.insert(RenderPlanItem::Surface(SurfaceRenderItem {
            identity: RenderItemIdentity { source_id: RenderSourceId(2), item_id: RenderItemId(2) },
            surface_id: 2,
            instance: RenderItemInstance {
                rect: RenderRect::default(),
                opacity: 1.0,
                clip_rect: None,
                z_index: 4,
                scene_role: RenderSceneRole::Desktop,
            },
        }));
        plan.insert(RenderPlanItem::Quad(QuadRenderItem {
            identity: RenderItemIdentity { source_id: RenderSourceId(1), item_id: RenderItemId(1) },
            content: QuadContent::SolidColor { color: RenderColor { r: 0, g: 0, b: 0, a: 255 } },
            instance: RenderItemInstance {
                rect: RenderRect::default(),
                opacity: 1.0,
                clip_rect: None,
                z_index: 1,
                scene_role: RenderSceneRole::Compositor,
            },
        }));

        plan.sort_by_z_index();

        let item_kinds = plan
            .iter_ordered()
            .map(|item| match item {
                RenderPlanItem::Surface(item) => format!("surface-{}", item.surface_id),
                RenderPlanItem::Quad(_) => "quad".to_owned(),
                RenderPlanItem::Backdrop(_) => "backdrop".to_owned(),
                RenderPlanItem::Cursor(_) => "cursor".to_owned(),
            })
            .collect::<Vec<_>>();
        assert_eq!(item_kinds, vec!["quad", "surface-2"]);
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
            identity: RenderItemIdentity { source_id: RenderSourceId(7), item_id: RenderItemId(7) },
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
            identity: RenderItemIdentity { source_id: RenderSourceId(8), item_id: RenderItemId(9) },
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
        assert_eq!(surface.source_id(), RenderSourceId(7));
        assert_eq!(surface.item_id(), RenderItemId(7));
    }

    #[test]
    fn cursor_surface_items_expose_underlying_surface_id() {
        let cursor = RenderPlanItem::Cursor(CursorRenderItem {
            identity: RenderItemIdentity {
                source_id: RenderSourceId(10),
                item_id: RenderItemId(11),
            },
            source: CursorRenderSource::Surface { surface_id: 42 },
            instance: RenderItemInstance {
                rect: RenderRect::default(),
                opacity: 1.0,
                clip_rect: None,
                z_index: i32::MAX,
                scene_role: RenderSceneRole::Cursor,
            },
        });

        assert_eq!(cursor.surface_id(), Some(42));
    }
}
