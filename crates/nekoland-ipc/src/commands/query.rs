use std::collections::BTreeMap;

use nekoland_ecs::components::OutputKind;
use nekoland_ecs::resources::ConfiguredAction;
use serde::{Deserialize, Serialize};

/// Read-only IPC queries exposed by `nekoland-msg query ...`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum QueryCommand {
    GetTree,
    GetOutputs,
    GetWorkspaces,
    GetCommands,
    GetConfig,
    GetClipboard,
    GetPrimarySelection,
}

/// Stable snapshot of one compositor output for IPC clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputSnapshot {
    pub name: String,
    pub kind: OutputKind,
    pub make: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
    pub scale: u32,
    pub viewport_origin_x: i64,
    pub viewport_origin_y: i64,
    pub current_workspace: Option<u32>,
}

/// Workspace state published through the IPC query cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub id: u32,
    pub name: String,
    pub active: bool,
}

/// Window tree entry returned by `get_tree`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub surface_id: u64,
    pub title: String,
    pub app_id: String,
    pub xwayland: bool,
    pub x11_window_id: Option<u32>,
    pub override_redirect: bool,
    pub x: i32,
    pub y: i32,
    pub scene_x: i64,
    pub scene_y: i64,
    pub screen_x: i32,
    pub screen_y: i32,
    pub width: u32,
    pub height: u32,
    pub state: String,
    pub workspace: Option<u32>,
    pub output: Option<String>,
    pub focused: bool,
    pub visible_in_viewport: bool,
}

/// Popup tree entry returned by `get_tree`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupSnapshot {
    pub surface_id: u64,
    pub parent_surface_id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub grab_active: bool,
    pub grab_serial: Option<u32>,
}

/// Top-level snapshot used by tree queries and as the baseline for subscription diffing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeSnapshot {
    pub frame: u64,
    pub focused_surface: Option<u64>,
    pub outputs: Vec<OutputSnapshot>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    pub windows: Vec<WindowSnapshot>,
    pub popups: Vec<PopupSnapshot>,
    pub render_order: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandStatusSnapshot {
    Launched { pid: u32 },
    Failed { error: String },
}

/// Historical record of one external command request and its observed result.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandSnapshot {
    pub frame: u64,
    pub uptime_millis: u128,
    pub origin: String,
    pub command: Option<Vec<String>>,
    pub candidates: Vec<Vec<String>>,
    pub status: Option<CommandStatusSnapshot>,
}

/// Config-facing output stanza published over IPC.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigOutputSnapshot {
    pub name: String,
    pub mode: String,
    pub scale: u32,
    pub enabled: bool,
}

/// User-facing config snapshot that merges the loaded file path, runtime reload state, and the
/// currently active normalized compositor config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSnapshot {
    pub path: Option<String>,
    pub loaded_from_disk: bool,
    pub successful_reloads: u64,
    pub last_reload_error: Option<String>,
    pub theme: String,
    pub cursor_theme: String,
    pub border_color: String,
    pub background_color: String,
    pub default_layout: String,
    pub focus_follows_mouse: bool,
    pub repeat_rate: u16,
    pub viewport_pan_modifiers: Vec<String>,
    pub command_history_limit: usize,
    pub startup_actions: Vec<ConfiguredAction>,
    pub xwayland_enabled: bool,
    pub outputs: Vec<ConfigOutputSnapshot>,
    pub keybindings: BTreeMap<String, Vec<ConfiguredAction>>,
}

/// Clipboard state exported through IPC.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub seat_name: Option<String>,
    pub mime_types: Vec<String>,
    pub owner: Option<SelectionOwnerSnapshot>,
    pub persisted_mime_types: Vec<String>,
}

/// Primary-selection state exported through IPC.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelectionSnapshot {
    pub seat_name: Option<String>,
    pub mime_types: Vec<String>,
    pub owner: Option<SelectionOwnerSnapshot>,
    pub persisted_mime_types: Vec<String>,
}

/// Normalized ownership marker for clipboard-style selections.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOwnerSnapshot {
    #[default]
    Client,
    Compositor,
}
