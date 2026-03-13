use std::collections::BTreeMap;
use std::fmt;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::{WindowLayout, WindowMode, WindowPolicy};

/// Output configuration after normalization from the on-disk config schema.
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

/// Keybinding action after normalization from the config schema.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ConfiguredKeybindingAction {
    Action(String),
    Command(Vec<String>),
}

impl ConfiguredKeybindingAction {
    /// Produces a human-readable label for diagnostics and command-history records.
    pub fn describe(&self) -> String {
        match self {
            Self::Action(action) => action.clone(),
            Self::Command(argv) => argv.join(" "),
        }
    }
}

/// XWayland-related runtime config.
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

/// Default layout policy selected for newly managed windows.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DefaultLayout {
    #[default]
    Floating,
    Maximized,
    Fullscreen,
    Tiling,
    Stacking,
}

impl DefaultLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Floating => "floating",
            Self::Maximized => "maximized",
            Self::Fullscreen => "fullscreen",
            Self::Tiling => "tiling",
            Self::Stacking => "stacking",
        }
    }

    pub const fn policy(self) -> WindowPolicy {
        match self {
            Self::Floating | Self::Stacking => {
                WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal)
            }
            Self::Tiling => WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal),
            Self::Maximized => WindowPolicy::new(WindowLayout::Floating, WindowMode::Maximized),
            Self::Fullscreen => WindowPolicy::new(WindowLayout::Floating, WindowMode::Fullscreen),
        }
    }
}

impl fmt::Display for DefaultLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One typed window rule layered on top of the global default window policy.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfiguredWindowRule {
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub layout: Option<WindowLayout>,
    pub mode: Option<WindowMode>,
}

impl ConfiguredWindowRule {
    pub fn matches(&self, app_id: &str, title: &str) -> bool {
        self.app_id.as_ref().is_none_or(|expected| expected == app_id)
            && self.title.as_ref().is_none_or(|expected| expected == title)
    }

    pub fn apply_to(&self, base: WindowPolicy) -> WindowPolicy {
        WindowPolicy {
            layout: self.layout.unwrap_or(base.layout),
            mode: self.mode.unwrap_or(base.mode),
        }
    }
}

/// Normalized compositor configuration consumed directly by runtime systems.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositorConfig {
    pub theme: String,
    pub cursor_theme: String,
    pub border_color: String,
    pub background_color: String,
    pub default_layout: DefaultLayout,
    pub window_rules: Vec<ConfiguredWindowRule>,
    pub focus_follows_mouse: bool,
    pub repeat_rate: u16,
    pub command_history_limit: usize,
    pub startup_commands: Vec<String>,
    pub outputs: Vec<ConfiguredOutput>,
    pub xwayland: XWaylandConfig,
    pub keybindings: BTreeMap<String, ConfiguredKeybindingAction>,
}

impl Default for CompositorConfig {
    fn default() -> Self {
        let mut keybindings = BTreeMap::new();
        keybindings.insert(
            "Super+Return".to_owned(),
            ConfiguredKeybindingAction::Command(vec!["foot".to_owned()]),
        );
        keybindings.insert(
            "Super+Space".to_owned(),
            ConfiguredKeybindingAction::Command(vec!["fuzzel".to_owned()]),
        );
        keybindings.insert(
            "Super+Q".to_owned(),
            ConfiguredKeybindingAction::Action("close-window".to_owned()),
        );

        Self {
            theme: "catppuccin-latte".to_owned(),
            cursor_theme: "default".to_owned(),
            border_color: "#5c7cfa".to_owned(),
            background_color: "#f5f7ff".to_owned(),
            default_layout: DefaultLayout::Floating,
            window_rules: Vec::new(),
            focus_follows_mouse: true,
            repeat_rate: 30,
            command_history_limit: DEFAULT_COMMAND_HISTORY_LIMIT,
            startup_commands: Vec::new(),
            outputs: vec![ConfiguredOutput::default()],
            xwayland: XWaylandConfig::default(),
            keybindings,
        }
    }
}

impl CompositorConfig {
    pub fn default_window_policy(&self) -> WindowPolicy {
        self.default_layout.policy()
    }

    pub fn resolve_window_policy(
        &self,
        app_id: &str,
        title: &str,
        override_redirect: bool,
    ) -> WindowPolicy {
        if override_redirect {
            return WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal);
        }

        self.window_rules
            .iter()
            .filter(|rule| rule.matches(app_id, title))
            .fold(self.default_window_policy(), |policy, rule| rule.apply_to(policy))
    }
}
