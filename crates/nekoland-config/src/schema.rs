use std::collections::BTreeMap;

use nekoland_ecs::resources::{
    CompositorConfig, ConfiguredKeybindingAction, ConfiguredOutput, ConfiguredWindowRule,
    DEFAULT_COMMAND_HISTORY_LIMIT, DefaultLayout, XWaylandConfig,
};
use serde::{Deserialize, Serialize};

use crate::{keybind_config::KeybindConfig, theme::Theme};

/// TOML-facing config schema loaded from disk before normalization into `CompositorConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NekolandConfigFile {
    pub theme: Theme,
    pub input: InputConfig,
    #[serde(default = "default_layout_name")]
    pub default_layout: DefaultLayout,
    #[serde(default)]
    pub window_rules: Vec<ConfiguredWindowRule>,
    #[serde(default)]
    pub ipc: IpcConfig,
    #[serde(default)]
    pub startup: StartupConfig,
    #[serde(default)]
    pub xwayland: XWaylandSection,
    pub outputs: Vec<OutputConfig>,
    pub keybinds: KeybindConfig,
}

impl Default for NekolandConfigFile {
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        bindings.insert(
            "Super+Return".to_owned(),
            ConfiguredKeybindingAction::Command(vec!["foot".to_owned()]),
        );
        bindings.insert(
            "Super+Space".to_owned(),
            ConfiguredKeybindingAction::Command(vec!["fuzzel".to_owned()]),
        );

        Self {
            theme: Theme::default(),
            input: InputConfig::default(),
            default_layout: default_layout_name(),
            window_rules: Vec::new(),
            ipc: IpcConfig::default(),
            startup: StartupConfig::default(),
            xwayland: XWaylandSection::default(),
            outputs: vec![OutputConfig::default()],
            keybinds: KeybindConfig { bindings },
        }
    }
}

/// Startup commands launched after the compositor finishes initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartupConfig {
    #[serde(default)]
    pub commands: Vec<String>,
}

/// IPC-specific settings that affect command history and related tooling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcConfig {
    #[serde(default = "default_command_history_limit")]
    pub command_history_limit: usize,
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self { command_history_limit: default_command_history_limit() }
    }
}

/// Disk schema for the XWayland section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XWaylandSection {
    #[serde(default = "default_xwayland_enabled")]
    pub enabled: bool,
}

impl Default for XWaylandSection {
    fn default() -> Self {
        Self { enabled: default_xwayland_enabled() }
    }
}

/// Input-related configuration loaded from the config file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputConfig {
    pub focus_follows_mouse: bool,
    pub repeat_rate: u16,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self { focus_follows_mouse: true, repeat_rate: 30 }
    }
}

/// Output stanza from the config file before it is normalized into `ConfiguredOutput`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputConfig {
    pub name: String,
    pub mode: String,
    pub scale: u32,
    pub enabled: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self { name: "eDP-1".to_owned(), mode: "1920x1080@60".to_owned(), scale: 1, enabled: true }
    }
}

/// Converts the deserialized config file into the normalized runtime config used throughout ECS.
impl From<NekolandConfigFile> for CompositorConfig {
    fn from(value: NekolandConfigFile) -> Self {
        Self {
            theme: value.theme.name,
            cursor_theme: value.theme.cursor_theme,
            border_color: value.theme.border_color,
            background_color: value.theme.background_color,
            default_layout: value.default_layout,
            window_rules: value.window_rules,
            focus_follows_mouse: value.input.focus_follows_mouse,
            repeat_rate: value.input.repeat_rate,
            command_history_limit: value.ipc.command_history_limit,
            startup_commands: value.startup.commands,
            xwayland: XWaylandConfig { enabled: value.xwayland.enabled },
            outputs: value
                .outputs
                .into_iter()
                .map(|output| ConfiguredOutput {
                    name: output.name,
                    mode: output.mode,
                    scale: output.scale.max(1),
                    enabled: output.enabled,
                })
                .collect(),
            keybindings: value.keybinds.bindings,
        }
    }
}

fn default_layout_name() -> DefaultLayout {
    DefaultLayout::Floating
}

fn default_command_history_limit() -> usize {
    DEFAULT_COMMAND_HISTORY_LIMIT
}

fn default_xwayland_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use nekoland_ecs::components::{WindowLayout, WindowMode, WindowPolicy};

    use super::NekolandConfigFile;

    #[test]
    fn parses_typed_window_rules_from_toml() {
        let config = toml::from_str::<NekolandConfigFile>(
            r##"
default_layout = "floating"

[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#112233"
background_color = "#ffffff"

[input]
focus_follows_mouse = true
repeat_rate = 30

[[window_rules]]
app_id = "org.nekoland.rules"
layout = "tiled"

[[window_rules]]
title = "Video"
mode = "fullscreen"

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Return" = ["foot"]
"##,
        )
        .expect("config should parse");

        let runtime = nekoland_ecs::resources::CompositorConfig::from(config);
        assert_eq!(runtime.window_rules.len(), 2);
        assert_eq!(
            runtime.resolve_window_policy("org.nekoland.rules", "Notes", false),
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal)
        );
        assert_eq!(
            runtime.resolve_window_policy("org.other.app", "Video", false),
            WindowPolicy::new(WindowLayout::Floating, WindowMode::Fullscreen)
        );
    }
}
