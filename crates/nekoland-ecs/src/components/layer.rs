use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerShellSurface {
    pub namespace: String,
    pub output: Option<String>,
    pub layer: LayerLevel,
    pub desired_width: u32,
    pub desired_height: u32,
    pub exclusive_zone: i32,
    pub margins: LayerMargins,
}

#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerAnchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl LayerAnchor {
    pub const fn centered() -> Self {
        Self { top: false, bottom: false, left: false, right: false }
    }
}

#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerMargins {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

#[derive(Component, Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerLevel {
    #[default]
    Background,
    Bottom,
    Top,
    Overlay,
}
