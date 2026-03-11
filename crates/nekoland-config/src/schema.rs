use std::collections::BTreeMap;

use nekoland_ecs::resources::{
    CompositorConfig, ConfiguredOutput, DEFAULT_COMMAND_HISTORY_LIMIT, ExternalCommandConfig,
    XWaylandConfig,
};
use serde::{Deserialize, Serialize};

use crate::{keybind_config::KeybindConfig, theme::Theme};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NekolandConfigFile {
    pub theme: Theme,
    pub input: InputConfig,
    #[serde(default = "default_layout_name")]
    pub default_layout: String,
    #[serde(default)]
    pub ipc: IpcConfig,
    #[serde(default)]
    pub startup: StartupConfig,
    #[serde(default)]
    pub commands: CommandConfig,
    #[serde(default)]
    pub xwayland: XWaylandSection,
    pub outputs: Vec<OutputConfig>,
    pub keybinds: KeybindConfig,
}

impl Default for NekolandConfigFile {
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        bindings.insert("Super+Return".to_owned(), "spawn-terminal".to_owned());
        bindings.insert("Super+Space".to_owned(), "launcher".to_owned());

        Self {
            theme: Theme::default(),
            input: InputConfig::default(),
            default_layout: default_layout_name(),
            ipc: IpcConfig::default(),
            startup: StartupConfig::default(),
            commands: CommandConfig::default(),
            xwayland: XWaylandSection::default(),
            outputs: vec![OutputConfig::default()],
            keybinds: KeybindConfig { bindings },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartupConfig {
    #[serde(default)]
    pub commands: Vec<String>,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandConfig {
    pub terminal: Option<String>,
    pub launcher: Option<String>,
    pub power_menu: Option<String>,
}

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

impl From<NekolandConfigFile> for CompositorConfig {
    fn from(value: NekolandConfigFile) -> Self {
        Self {
            theme: value.theme.name,
            cursor_theme: value.theme.cursor_theme,
            border_color: value.theme.border_color,
            background_color: value.theme.background_color,
            default_layout: value.default_layout,
            focus_follows_mouse: value.input.focus_follows_mouse,
            repeat_rate: value.input.repeat_rate,
            command_history_limit: value.ipc.command_history_limit,
            startup_commands: value.startup.commands,
            commands: ExternalCommandConfig {
                terminal: value.commands.terminal,
                launcher: value.commands.launcher,
                power_menu: value.commands.power_menu,
            },
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

fn default_layout_name() -> String {
    "floating".to_owned()
}

fn default_command_history_limit() -> usize {
    DEFAULT_COMMAND_HISTORY_LIMIT
}

fn default_xwayland_enabled() -> bool {
    true
}
