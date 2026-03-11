use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct XdgWindow {
    pub app_id: String,
    pub title: String,
    pub last_acked_configure: Option<u32>,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowState {
    #[default]
    Tiled,
    Floating,
    Maximized,
    Fullscreen,
    Hidden,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayoutSlot {
    pub workspace: u32,
    pub column: u16,
    pub row: u16,
}
