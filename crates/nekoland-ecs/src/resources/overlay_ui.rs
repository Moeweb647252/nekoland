use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{RenderColor, RenderRect};

/// Stable caller-defined identifier for one overlay UI primitive.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct OverlayUiPrimitiveId(pub String);

impl OverlayUiPrimitiveId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for OverlayUiPrimitiveId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for OverlayUiPrimitiveId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Coarse overlay UI layer used to derive ordering bands inside one output-local overlay scene.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum OverlayUiLayer {
    Background,
    #[default]
    Main,
    Foreground,
}

impl OverlayUiLayer {
    pub const fn z_index_bias(self) -> i32 {
        match self {
            Self::Background => -1_000_000,
            Self::Main => 0,
            Self::Foreground => 1_000_000,
        }
    }
}

/// One frame-local panel primitive in the overlay HUD.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OverlayUiPanel {
    pub id: OverlayUiPrimitiveId,
    pub layer: OverlayUiLayer,
    pub rect: RenderRect,
    pub clip_rect: Option<RenderRect>,
    pub color: RenderColor,
    pub opacity: f32,
    pub z_index: i32,
}

/// One frame-local text primitive in the overlay HUD.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OverlayUiText {
    pub id: OverlayUiPrimitiveId,
    pub layer: OverlayUiLayer,
    pub x: i32,
    pub y: i32,
    pub clip_rect: Option<RenderRect>,
    pub text: String,
    pub font_size: f32,
    pub color: RenderColor,
    pub opacity: f32,
    pub z_index: i32,
}

/// One frame-local overlay UI primitive.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum OverlayUiPrimitive {
    Panel(OverlayUiPanel),
    Text(OverlayUiText),
}

impl OverlayUiPrimitive {
    pub fn id(&self) -> &OverlayUiPrimitiveId {
        match self {
            Self::Panel(panel) => &panel.id,
            Self::Text(text) => &text.id,
        }
    }

    pub fn layer(&self) -> OverlayUiLayer {
        match self {
            Self::Panel(panel) => panel.layer,
            Self::Text(text) => text.layer,
        }
    }

    pub fn z_index(&self) -> i32 {
        match self {
            Self::Panel(panel) => panel.z_index,
            Self::Text(text) => text.z_index,
        }
    }
}

/// One output-local collection of overlay UI primitives for the current frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OverlayUiOutputFrame {
    pub primitives: Vec<OverlayUiPrimitive>,
}

/// Frame-local overlay UI primitives produced by shell/main-world systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OverlayUiFrame {
    pub outputs: BTreeMap<OutputId, OverlayUiOutputFrame>,
}

impl OverlayUiFrame {
    pub fn clear(&mut self) {
        self.outputs.clear();
    }

    pub fn output(&mut self, output_id: OutputId) -> OverlayUiOutputBuilder<'_> {
        let output = self.outputs.entry(output_id).or_default();
        OverlayUiOutputBuilder { primitives: &mut output.primitives }
    }
}

pub struct OverlayUiOutputBuilder<'a> {
    primitives: &'a mut Vec<OverlayUiPrimitive>,
}

impl OverlayUiOutputBuilder<'_> {
    pub fn panel(
        &mut self,
        id: impl Into<OverlayUiPrimitiveId>,
        layer: OverlayUiLayer,
        rect: RenderRect,
        clip_rect: Option<RenderRect>,
        color: RenderColor,
        opacity: f32,
        z_index: i32,
    ) -> &mut Self {
        self.upsert(OverlayUiPrimitive::Panel(OverlayUiPanel {
            id: id.into(),
            layer,
            rect,
            clip_rect,
            color,
            opacity: opacity.clamp(0.0, 1.0),
            z_index,
        }));
        self
    }

    pub fn text(
        &mut self,
        id: impl Into<OverlayUiPrimitiveId>,
        layer: OverlayUiLayer,
        x: i32,
        y: i32,
        clip_rect: Option<RenderRect>,
        text: impl Into<String>,
        font_size: f32,
        color: RenderColor,
        opacity: f32,
        z_index: i32,
    ) -> &mut Self {
        self.upsert(OverlayUiPrimitive::Text(OverlayUiText {
            id: id.into(),
            layer,
            x,
            y,
            clip_rect,
            text: text.into(),
            font_size: font_size.max(1.0),
            color,
            opacity: opacity.clamp(0.0, 1.0),
            z_index,
        }));
        self
    }

    fn upsert(&mut self, primitive: OverlayUiPrimitive) {
        let primitive_id = primitive.id().clone();
        self.primitives.retain(|existing| existing.id() != &primitive_id);
        self.primitives.push(primitive);
    }
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;
    use crate::resources::{OverlayUiFrame, OverlayUiLayer, RenderColor, RenderRect};

    #[test]
    fn output_builder_keeps_last_primitive_for_same_id() {
        let mut frame = OverlayUiFrame::default();
        frame
            .output(OutputId(3))
            .panel(
                "panel",
                OverlayUiLayer::Main,
                RenderRect { x: 0, y: 0, width: 10, height: 10 },
                None,
                RenderColor { r: 1, g: 2, b: 3, a: 255 },
                1.0,
                0,
            )
            .panel(
                "panel",
                OverlayUiLayer::Foreground,
                RenderRect { x: 5, y: 6, width: 20, height: 30 },
                None,
                RenderColor { r: 4, g: 5, b: 6, a: 255 },
                0.5,
                7,
            );

        let output = frame.outputs.get(&OutputId(3)).expect("overlay UI output should exist");
        assert_eq!(output.primitives.len(), 1);
        let crate::resources::OverlayUiPrimitive::Panel(panel) = &output.primitives[0] else {
            panic!("expected panel primitive");
        };
        assert_eq!(panel.rect.x, 5);
        assert_eq!(panel.layer, OverlayUiLayer::Foreground);
    }
}
