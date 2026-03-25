//! Read-only IPC query commands and serialized snapshot payloads.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use nekoland_config::resources::ConfiguredAction;
use nekoland_ecs::components::{OutputKind, SeatId};
use serde::{Deserialize, Serialize};

/// Read-only IPC queries exposed by `nekoland-msg query ...`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum QueryCommand {
    GetTree,
    GetOutputs,
    GetWorkspaces,
    GetWindows,
    GetKeyboardLayouts,
    GetCommands,
    GetConfig,
    GetClipboard,
    GetPrimarySelection,
    GetPresentAudit,
}

/// Stable snapshot of one compositor output for IPC clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputSnapshot {
    pub name: String,
    pub kind: OutputKind,
    pub make: String,
    pub model: String,
    pub connected: bool,
    pub enabled: bool,
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
    pub scale: u32,
    pub x: i32,
    pub y: i32,
    pub viewport_origin_x: i64,
    pub viewport_origin_y: i64,
    pub work_area_x: i32,
    pub work_area_y: i32,
    pub work_area_width: u32,
    pub work_area_height: u32,
    pub mode: String,
    pub current_workspace: Option<u32>,
}

/// Workspace state published through the IPC query cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub id: u32,
    pub idx: u32,
    pub name: String,
    pub active: bool,
    pub focused: bool,
    pub occupied: bool,
    pub urgent: bool,
    pub output: Option<String>,
}

/// One keyboard layout entry exported through IPC.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardLayoutEntrySnapshot {
    pub name: String,
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: String,
}

/// Runtime keyboard-layout state for the compositor seat.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardLayoutsSnapshot {
    pub seat_id: SeatId,
    pub seat_name: String,
    pub active_index: usize,
    pub active_name: String,
    pub layouts: Vec<KeyboardLayoutEntrySnapshot>,
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
    pub role: String,
    pub layout: String,
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
    pub render_index: Option<usize>,
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
    pub fps_hud_enabled: bool,
    pub default_layout: String,
    pub focus_follows_mouse: bool,
    pub repeat_rate: u16,
    pub configured_keyboard_layout: String,
    pub keyboard_layouts: Vec<KeyboardLayoutEntrySnapshot>,
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
    pub seat_id: Option<SeatId>,
    pub seat_name: Option<String>,
    pub mime_types: Vec<String>,
    pub owner: Option<SelectionOwnerSnapshot>,
    pub persisted_mime_types: Vec<String>,
}

/// Primary-selection state exported through IPC.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelectionSnapshot {
    pub seat_id: Option<SeatId>,
    pub seat_name: Option<String>,
    pub mime_types: Vec<String>,
    pub owner: Option<SelectionOwnerSnapshot>,
    pub persisted_mime_types: Vec<String>,
}

/// One output-local present-audit element exposed for debug queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PresentAuditElementSnapshot {
    pub surface_id: u64,
    pub kind: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z_index: i32,
    pub opacity: f32,
}

/// Output-local present-audit snapshot exposed through `query present-audit`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PresentAuditOutputSnapshot {
    pub output_name: String,
    pub frame: u64,
    pub uptime_millis: u64,
    pub elements: Vec<PresentAuditElementSnapshot>,
}

/// Normalized ownership marker for clipboard-style selections.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOwnerSnapshot {
    #[default]
    Client,
    Compositor,
}
