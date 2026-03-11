use std::collections::BTreeMap;

use nekoland_ecs::components::OutputKind;
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub id: u32,
    pub name: String,
    pub active: bool,
}

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
    pub width: u32,
    pub height: u32,
    pub state: String,
    pub workspace: Option<u32>,
    pub focused: bool,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandSnapshot {
    pub frame: u64,
    pub uptime_millis: u128,
    pub origin: String,
    pub command: Option<Vec<String>>,
    pub candidates: Vec<Vec<String>>,
    pub status: Option<CommandStatusSnapshot>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigCommandSnapshot {
    pub terminal: Option<String>,
    pub launcher: Option<String>,
    pub power_menu: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigOutputSnapshot {
    pub name: String,
    pub mode: String,
    pub scale: u32,
    pub enabled: bool,
}

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
    pub command_history_limit: usize,
    pub startup_commands: Vec<String>,
    pub xwayland_enabled: bool,
    pub outputs: Vec<ConfigOutputSnapshot>,
    pub commands: ConfigCommandSnapshot,
    pub keybindings: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub seat_name: Option<String>,
    pub mime_types: Vec<String>,
    pub owner: Option<SelectionOwnerSnapshot>,
    pub persisted_mime_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelectionSnapshot {
    pub seat_name: Option<String>,
    pub mime_types: Vec<String>,
    pub owner: Option<SelectionOwnerSnapshot>,
    pub persisted_mime_types: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOwnerSnapshot {
    #[default]
    Client,
    Compositor,
}
