//! IPC command and snapshot types shared by the server and the `nekoland-msg` CLI.

pub mod action;
pub mod output;
pub mod popup;
pub mod query;
pub mod window;
pub mod workspace;

pub use action::ActionCommand;
pub use output::OutputCommand;
pub use popup::PopupCommand;
pub use query::{
    ClipboardSnapshot, CommandSnapshot, CommandStatusSnapshot, ConfigOutputSnapshot,
    ConfigSnapshot, OutputSnapshot, PopupSnapshot, PrimarySelectionSnapshot, QueryCommand,
    SelectionOwnerSnapshot, TreeSnapshot, WindowSnapshot, WorkspaceSnapshot,
};
pub use window::{SplitAxis, WindowCommand};
pub use workspace::WorkspaceCommand;
