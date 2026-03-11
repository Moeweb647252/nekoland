use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfiguredOutput {
    pub name: String,
    pub mode: String,
    pub scale: u32,
    pub enabled: bool,
}

impl Default for ConfiguredOutput {
    fn default() -> Self {
        Self { name: "eDP-1".to_owned(), mode: "1920x1080@60".to_owned(), scale: 1, enabled: true }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandConfig {
    pub terminal: Option<String>,
    pub launcher: Option<String>,
    pub power_menu: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct XWaylandConfig {
    pub enabled: bool,
}

impl Default for XWaylandConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub const DEFAULT_COMMAND_HISTORY_LIMIT: usize = 64;

#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositorConfig {
    pub theme: String,
    pub cursor_theme: String,
    pub border_color: String,
    pub background_color: String,
    pub default_layout: String,
    pub focus_follows_mouse: bool,
    pub repeat_rate: u16,
    pub command_history_limit: usize,
    pub startup_commands: Vec<String>,
    pub outputs: Vec<ConfiguredOutput>,
    pub commands: ExternalCommandConfig,
    pub xwayland: XWaylandConfig,
    pub keybindings: BTreeMap<String, String>,
}

impl Default for CompositorConfig {
    fn default() -> Self {
        let mut keybindings = BTreeMap::new();
        keybindings.insert("Super+Return".to_owned(), "spawn-terminal".to_owned());
        keybindings.insert("Super+Q".to_owned(), "close-window".to_owned());

        Self {
            theme: "catppuccin-latte".to_owned(),
            cursor_theme: "default".to_owned(),
            border_color: "#5c7cfa".to_owned(),
            background_color: "#f5f7ff".to_owned(),
            default_layout: "floating".to_owned(),
            focus_follows_mouse: true,
            repeat_rate: 30,
            command_history_limit: DEFAULT_COMMAND_HISTORY_LIMIT,
            startup_commands: Vec::new(),
            outputs: vec![ConfiguredOutput::default()],
            commands: ExternalCommandConfig::default(),
            xwayland: XWaylandConfig::default(),
            keybindings,
        }
    }
}
