//! TOML-facing config schema loaded from disk before normalization into runtime resources.
//!
//! Most public fields mirror the disk representation directly, so type-level documentation is used
//! in place of repetitive field-by-field comments.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use crate::resources::{
    CompositorConfig, ConfiguredAction, ConfiguredKeyboardLayout, ConfiguredOutput,
    ConfiguredWindowRule, DEFAULT_COMMAND_HISTORY_LIMIT, DefaultLayout, XWaylandConfig,
};
use nekoland_ecs::resources::ModifierMask;
use serde::{Deserialize, Serialize};

use crate::{
    action_config::{ActionListConfig, ConfiguredActionConfig, KeybindEntryConfig},
    keybind_config::KeybindConfig,
    theme::Theme,
};

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
            ActionListConfig::One(ConfiguredActionConfig::Exec { exec: vec!["foot".to_owned()] }),
        );
        bindings.insert(
            "Super+Space".to_owned(),
            ActionListConfig::One(ConfiguredActionConfig::Exec { exec: vec!["fuzzel".to_owned()] }),
        );
        bindings.insert(
            "Super+Q".to_owned(),
            ActionListConfig::One(ConfiguredActionConfig::Close { close: true }),
        );
        bindings.insert(
            "Super+Alt".to_owned(),
            ActionListConfig::One(ConfiguredActionConfig::ViewportPanMode {
                viewport_pan_mode: true,
            }),
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

/// Startup actions applied after the compositor finishes initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartupConfig {
    #[serde(default)]
    pub actions: Vec<ConfiguredActionConfig>,
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
    #[serde(default)]
    pub keyboard: KeyboardConfig,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self { focus_follows_mouse: true, repeat_rate: 30, keyboard: KeyboardConfig::default() }
    }
}

/// Keyboard-layout configuration loaded from disk before normalization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardConfig {
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default = "default_keyboard_layouts")]
    pub layouts: Vec<KeyboardLayoutConfig>,
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self { current: None, layouts: default_keyboard_layouts() }
    }
}

/// One keyboard layout stanza inside `[input.keyboard]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardLayoutConfig {
    #[serde(default)]
    pub name: Option<String>,
    pub layout: String,
    #[serde(default)]
    pub rules: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub options: String,
}

impl Default for KeyboardLayoutConfig {
    fn default() -> Self {
        Self {
            name: Some("us".to_owned()),
            layout: "us".to_owned(),
            rules: String::new(),
            model: String::new(),
            variant: String::new(),
            options: String::new(),
        }
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
impl TryFrom<NekolandConfigFile> for CompositorConfig {
    type Error = String;

    fn try_from(value: NekolandConfigFile) -> Result<Self, Self::Error> {
        let mut viewport_pan_modifiers = CompositorConfig::default().viewport_pan_modifiers;
        let mut viewport_pan_mode_binding = None::<String>;
        let mut keybindings = BTreeMap::new();
        let (keyboard_layouts, current_keyboard_layout) =
            normalize_keyboard_layouts(value.input.keyboard)?;

        for (binding, actions) in value.keybinds.bindings {
            match actions.into_keybind_entry()? {
                KeybindEntryConfig::Actions(actions) => {
                    keybindings.insert(binding, actions);
                }
                KeybindEntryConfig::ViewportPanMode => {
                    if let Some(previous) = viewport_pan_mode_binding.replace(binding.clone()) {
                        return Err(format!(
                            "viewport pan mode binding may only be configured once, found `{previous}` and `{binding}`"
                        ));
                    }
                    viewport_pan_modifiers = parse_viewport_pan_mode_binding(&binding)?;
                }
            }
        }

        Ok(Self {
            theme: value.theme.name,
            cursor_theme: value.theme.cursor_theme,
            border_color: value.theme.border_color,
            background_color: value.theme.background_color,
            default_layout: value.default_layout,
            window_rules: value.window_rules,
            focus_follows_mouse: value.input.focus_follows_mouse,
            repeat_rate: value.input.repeat_rate,
            current_keyboard_layout,
            keyboard_layouts,
            viewport_pan_modifiers,
            command_history_limit: value.ipc.command_history_limit,
            startup_actions: value
                .startup
                .actions
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<ConfiguredAction>, _>>()?,
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
            keybindings,
        })
    }
}

fn normalize_keyboard_layouts(
    keyboard: KeyboardConfig,
) -> Result<(Vec<ConfiguredKeyboardLayout>, String), String> {
    let KeyboardConfig { current, layouts } = keyboard;
    let layouts = if layouts.is_empty() {
        vec![ConfiguredKeyboardLayout::default()]
    } else {
        layouts
            .into_iter()
            .map(|layout| {
                let name = layout.name.unwrap_or_else(|| layout.layout.clone());
                let name = name.trim().to_owned();
                if name.is_empty() {
                    return Err("keyboard layout name must not be empty".to_owned());
                }
                if layout.layout.trim().is_empty() {
                    return Err(format!("keyboard layout `{name}` must set a non-empty layout"));
                }

                Ok(ConfiguredKeyboardLayout {
                    name,
                    rules: layout.rules,
                    model: layout.model,
                    layout: layout.layout,
                    variant: layout.variant,
                    options: layout.options,
                })
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut seen = std::collections::BTreeSet::new();
    for layout in &layouts {
        if !seen.insert(layout.name.clone()) {
            return Err(format!(
                "keyboard layout names must be unique, duplicate `{}`",
                layout.name
            ));
        }
    }

    let current = current.unwrap_or_else(|| {
        layouts.first().map(|layout| layout.name.clone()).unwrap_or("us".to_owned())
    });
    if layouts.iter().all(|layout| layout.name != current) {
        return Err(format!(
            "keyboard current layout `{current}` was not found in input.keyboard.layouts"
        ));
    }

    Ok((layouts, current))
}

fn default_layout_name() -> DefaultLayout {
    DefaultLayout::Floating
}

fn default_keyboard_layouts() -> Vec<KeyboardLayoutConfig> {
    vec![KeyboardLayoutConfig::default()]
}

fn parse_viewport_pan_mode_binding(binding: &str) -> Result<ModifierMask, String> {
    ModifierMask::from_config_tokens(
        binding.split('+').map(str::trim).filter(|token| !token.is_empty()),
    )
    .map_err(|error| format!("invalid viewport pan mode binding `{binding}`: {error}"))
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
    use nekoland_ecs::resources::ModifierMask;

    use super::NekolandConfigFile;

    #[test]
    fn parses_typed_window_rules_from_toml() {
        let Ok(config) = toml::from_str::<NekolandConfigFile>(
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

[[window_rules]]
app_id = "swaybg"
background = "eDP-1"

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Return" = { exec = ["foot"] }
"Super+Alt" = { viewport_pan_mode = true }
"##,
        ) else {
            panic!("config should parse");
        };

        let Ok(runtime) = crate::resources::CompositorConfig::try_from(config) else {
            panic!("config should normalize");
        };
        assert_eq!(runtime.window_rules.len(), 3);
        assert_eq!(
            runtime.resolve_window_policy("org.nekoland.rules", "Notes", false),
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal)
        );
        assert_eq!(
            runtime.resolve_window_policy("org.other.app", "Video", false),
            WindowPolicy::new(WindowLayout::Floating, WindowMode::Fullscreen)
        );
        assert_eq!(
            runtime.resolve_window_background("swaybg", "Wallpaper", false),
            Some(nekoland_ecs::selectors::OutputName::from("eDP-1"))
        );
        assert_eq!(runtime.viewport_pan_modifiers, ModifierMask::new(false, true, false, true));
    }

    #[test]
    fn keybinding_viewport_pan_mode_normalizes_into_runtime_mask() {
        let Ok(config) = toml::from_str::<NekolandConfigFile>(
            r##"
[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#112233"
background_color = "#ffffff"

[input]
focus_follows_mouse = true
repeat_rate = 30

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Return" = { exec = ["foot"] }
"Ctrl+Shift" = { viewport_pan_mode = true }
"##,
        ) else {
            panic!("config should parse");
        };

        let Ok(runtime) = crate::resources::CompositorConfig::try_from(config) else {
            panic!("config should normalize");
        };
        assert_eq!(runtime.viewport_pan_modifiers, ModifierMask::new(true, false, true, false));
    }

    #[test]
    fn viewport_pan_mode_binding_rejects_non_modifier_keys() {
        let Ok(config) = toml::from_str::<NekolandConfigFile>(
            r##"
[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#112233"
background_color = "#ffffff"

[input]
focus_follows_mouse = true
repeat_rate = 30

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+H" = { viewport_pan_mode = true }
"##,
        ) else {
            panic!("config should parse");
        };

        assert_eq!(
            crate::resources::CompositorConfig::try_from(config),
            Err("invalid viewport pan mode binding `Super+H`: unsupported modifier `H`".to_owned())
        );
    }

    #[test]
    fn keyboard_layouts_normalize_and_validate_current_layout() {
        let Ok(config) = toml::from_str::<NekolandConfigFile>(
            r##"
[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#112233"
background_color = "#ffffff"

[input]
focus_follows_mouse = true
repeat_rate = 30

[input.keyboard]
current = "de"

[[input.keyboard.layouts]]
layout = "us"

[[input.keyboard.layouts]]
name = "de"
layout = "de"
variant = "nodeadkeys"

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Return" = { exec = ["foot"] }
"##,
        ) else {
            panic!("config should parse");
        };

        let Ok(runtime) = crate::resources::CompositorConfig::try_from(config) else {
            panic!("config should normalize");
        };

        assert_eq!(runtime.current_keyboard_layout, "de");
        assert_eq!(runtime.keyboard_layouts.len(), 2);
        assert_eq!(runtime.keyboard_layouts[0].name, "us");
        assert_eq!(runtime.keyboard_layouts[1].name, "de");
        assert_eq!(runtime.keyboard_layouts[1].variant, "nodeadkeys");
    }
}
