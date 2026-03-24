use std::collections::BTreeMap;
use std::fmt;

use bevy_ecs::prelude::Resource;
use nekoland_ecs::components::{WindowLayout, WindowMode, WindowPolicy};
use nekoland_ecs::resources::{ModifierMask, SplitAxis};
use nekoland_ecs::selectors::{OutputName, WorkspaceLookup, WorkspaceSelector};
use serde::{Deserialize, Serialize};

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

/// Keyboard layout configuration after normalization from the on-disk config schema.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfiguredKeyboardLayout {
    pub name: String,
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
}

impl Default for ConfiguredKeyboardLayout {
    fn default() -> Self {
        Self {
            name: "us".to_owned(),
            rules: String::new(),
            model: String::new(),
            layout: "us".to_owned(),
            variant: String::new(),
            options: String::new(),
        }
    }
}

/// Configured action after normalization from the config schema.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ConfiguredAction {
    Exec { argv: Vec<String> },
    CloseFocusedWindow,
    MoveFocusedWindow { x: isize, y: isize },
    ResizeFocusedWindow { width: u32, height: u32 },
    SplitFocusedWindow { axis: SplitAxis },
    BackgroundFocusedWindow { output: OutputName },
    ClearFocusedWindowBackground,
    SwitchWorkspace { workspace: WorkspaceLookup },
    CreateWorkspace { workspace: WorkspaceLookup },
    DestroyWorkspace { workspace: WorkspaceSelector },
    EnableOutput { output: OutputName },
    DisableOutput { output: OutputName },
    ConfigureOutput { output: OutputName, mode: String, scale: Option<u32> },
    PanViewport { delta_x: isize, delta_y: isize },
    MoveViewport { x: isize, y: isize },
    CenterViewportOnFocusedWindow,
}

impl ConfiguredAction {
    /// Produces a human-readable label for diagnostics and command-history records.
    pub fn describe(&self) -> String {
        match self {
            Self::Exec { argv } => argv.join(" "),
            Self::CloseFocusedWindow => "close-focused-window".to_owned(),
            Self::MoveFocusedWindow { x, y } => format!("move-focused-window {x} {y}"),
            Self::ResizeFocusedWindow { width, height } => {
                format!("resize-focused-window {width} {height}")
            }
            Self::SplitFocusedWindow { axis } => {
                format!("split-focused-window {}", split_axis_label(*axis))
            }
            Self::BackgroundFocusedWindow { output } => {
                format!("background-focused-window {}", output.as_str())
            }
            Self::ClearFocusedWindowBackground => "clear-focused-window-background".to_owned(),
            Self::SwitchWorkspace { workspace } => {
                format!("switch-workspace {}", workspace_lookup_label(workspace))
            }
            Self::CreateWorkspace { workspace } => {
                format!("create-workspace {}", workspace_lookup_label(workspace))
            }
            Self::DestroyWorkspace { workspace } => {
                format!("destroy-workspace {}", workspace_selector_label(workspace))
            }
            Self::EnableOutput { output } => format!("enable-output {}", output.as_str()),
            Self::DisableOutput { output } => format!("disable-output {}", output.as_str()),
            Self::ConfigureOutput { output, mode, scale } => match scale {
                Some(scale) => format!("configure-output {} {mode} {scale}", output.as_str()),
                None => format!("configure-output {} {mode}", output.as_str()),
            },
            Self::PanViewport { delta_x, delta_y } => {
                format!("pan-viewport {delta_x} {delta_y}")
            }
            Self::MoveViewport { x, y } => format!("move-viewport {x} {y}"),
            Self::CenterViewportOnFocusedWindow => "center-viewport-on-focused-window".to_owned(),
        }
    }
}

pub fn describe_action_sequence(actions: &[ConfiguredAction]) -> String {
    actions.iter().map(ConfiguredAction::describe).collect::<Vec<_>>().join(" ; ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRuleContext<'a> {
    pub app_id: &'a str,
    pub title: &'a str,
    pub bypass_window_rules: bool,
    pub helper_surface: bool,
    pub prefer_floating: bool,
}

fn split_axis_label(axis: SplitAxis) -> &'static str {
    match axis {
        SplitAxis::Horizontal => "horizontal",
        SplitAxis::Vertical => "vertical",
    }
}

fn workspace_lookup_label(workspace: &WorkspaceLookup) -> String {
    match workspace {
        WorkspaceLookup::Id(id) => id.0.to_string(),
        WorkspaceLookup::Name(name) => name.as_str().to_owned(),
    }
}

fn workspace_selector_label(workspace: &WorkspaceSelector) -> String {
    match workspace {
        WorkspaceSelector::Active => "active".to_owned(),
        WorkspaceSelector::Id(id) => id.0.to_string(),
        WorkspaceSelector::Name(name) => name.as_str().to_owned(),
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
    pub background: Option<OutputName>,
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
    pub current_keyboard_layout: String,
    pub keyboard_layouts: Vec<ConfiguredKeyboardLayout>,
    pub viewport_pan_modifiers: ModifierMask,
    pub command_history_limit: usize,
    pub startup_actions: Vec<ConfiguredAction>,
    pub outputs: Vec<ConfiguredOutput>,
    pub xwayland: XWaylandConfig,
    pub keybindings: BTreeMap<String, Vec<ConfiguredAction>>,
}

impl Default for CompositorConfig {
    fn default() -> Self {
        let mut keybindings = BTreeMap::new();
        keybindings.insert(
            "Super+Return".to_owned(),
            vec![ConfiguredAction::Exec { argv: vec!["foot".to_owned()] }],
        );
        keybindings.insert(
            "Super+Space".to_owned(),
            vec![ConfiguredAction::Exec { argv: vec!["fuzzel".to_owned()] }],
        );
        keybindings.insert("Super+Q".to_owned(), vec![ConfiguredAction::CloseFocusedWindow]);

        Self {
            theme: "catppuccin-latte".to_owned(),
            cursor_theme: "default".to_owned(),
            border_color: "#5c7cfa".to_owned(),
            background_color: "#f5f7ff".to_owned(),
            default_layout: DefaultLayout::Floating,
            window_rules: Vec::new(),
            focus_follows_mouse: true,
            repeat_rate: 30,
            current_keyboard_layout: "us".to_owned(),
            keyboard_layouts: vec![ConfiguredKeyboardLayout::default()],
            viewport_pan_modifiers: ModifierMask::new(false, true, false, true),
            command_history_limit: DEFAULT_COMMAND_HISTORY_LIMIT,
            startup_actions: Vec::new(),
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
        self.resolve_window_policy_with_context(WindowRuleContext {
            app_id,
            title,
            bypass_window_rules: override_redirect,
            helper_surface: false,
            prefer_floating: override_redirect,
        })
    }

    pub fn resolve_window_policy_with_context(
        &self,
        context: WindowRuleContext<'_>,
    ) -> WindowPolicy {
        if context.bypass_window_rules {
            return WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal);
        }

        let mut policy = self
            .window_rules
            .iter()
            .filter(|rule| rule.matches(context.app_id, context.title))
            .fold(self.default_window_policy(), |policy, rule| rule.apply_to(policy));
        if context.prefer_floating {
            policy.layout = WindowLayout::Floating;
        }
        policy
    }

    pub fn resolve_window_background(
        &self,
        app_id: &str,
        title: &str,
        override_redirect: bool,
    ) -> Option<OutputName> {
        self.resolve_window_background_with_context(WindowRuleContext {
            app_id,
            title,
            bypass_window_rules: override_redirect,
            helper_surface: false,
            prefer_floating: override_redirect,
        })
    }

    pub fn resolve_window_background_with_context(
        &self,
        context: WindowRuleContext<'_>,
    ) -> Option<OutputName> {
        if context.bypass_window_rules {
            return None;
        }

        self.window_rules
            .iter()
            .filter(|rule| rule.matches(context.app_id, context.title))
            .fold(None, |background, rule| rule.background.clone().or(background))
    }
}
