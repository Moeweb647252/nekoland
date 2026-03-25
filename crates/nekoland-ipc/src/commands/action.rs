use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FpsHudMode {
    On,
    Off,
    Toggle,
}

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
    FpsHud { mode: FpsHudMode },
    ReloadConfig,
    Quit,
    PowerOffMonitors,
    PowerOnMonitors,
}
