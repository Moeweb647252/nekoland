use serde::{Deserialize, Serialize};

/// Theme section loaded from the config file before normalization into `CompositorConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Theme {
    /// Human-readable theme name exposed through config snapshots and diagnostics.
    pub name: String,
    /// Cursor theme requested by compositor-owned cursor rendering paths.
    pub cursor_theme: String,
    /// Font family used by compositor-owned overlay text such as HUDs and switchers.
    #[serde(default = "default_overlay_font_family")]
    pub overlay_font_family: String,
    /// Accent color used by server-side border rendering.
    pub border_color: String,
    /// Background color used when clearing compositor-owned render targets.
    pub background_color: String,
}

fn default_overlay_font_family() -> String {
    "Noto Sans".to_owned()
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "catppuccin-latte".to_owned(),
            cursor_theme: "default".to_owned(),
            overlay_font_family: default_overlay_font_family(),
            border_color: "#5c7cfa".to_owned(),
            background_color: "#f5f7ff".to_owned(),
        }
    }
}
