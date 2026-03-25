//! Shell-style imperative action commands accepted over IPC.

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// Higher-level shell-style actions accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionCommand {
    FocusWorkspace { workspace: String },
    FocusWindow { id: u64 },
    CloseWindow { id: u64 },
    Spawn { command: Vec<String> },
    SwitchKeyboardLayoutNext,
    SwitchKeyboardLayoutPrev,
    SwitchKeyboardLayoutByName { name: String },
    SwitchKeyboardLayoutByIndex { index: usize },
    ReloadConfig,
    Quit,
    PowerOffMonitors,
    PowerOnMonitors,
}
