//! IPC command and snapshot types shared by the server and the `nekoland-msg` CLI.

/// Shell-style imperative actions.
pub mod action;
/// Output-management commands.
pub mod output;
/// Popup-management commands.
pub mod popup;
/// Read-only query commands and snapshot payloads.
pub mod query;
/// Tiling-management commands.
pub mod tiling;
/// Window-management commands.
pub mod window;
/// Workspace-management commands.
pub mod workspace;

pub use action::{ActionCommand, FpsHudMode};
pub use output::OutputCommand;
pub use popup::PopupCommand;
pub use query::{
    ClipboardSnapshot, CommandSnapshot, CommandStatusSnapshot, ConfigOutputSnapshot,
    ConfigSnapshot, KeyboardLayoutEntrySnapshot, KeyboardLayoutsSnapshot, OutputSnapshot,
    PopupSnapshot, PresentAuditElementSnapshot, PresentAuditOutputSnapshot,
    PrimarySelectionSnapshot, QueryCommand, SelectionOwnerSnapshot, TreeSnapshot, WindowSnapshot,
    WorkspaceSnapshot,
};
pub use tiling::{HorizontalDirection, TilingCommand, TilingPanDirection, VerticalDirection};
pub use window::WindowCommand;
pub use workspace::WorkspaceCommand;
