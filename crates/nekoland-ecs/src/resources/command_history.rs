//! External command execution history retained for IPC and diagnostics.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

const DEFAULT_COMMAND_HISTORY_LIMIT: usize = 64;

/// Final observed status for one external command request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandExecutionStatus {
    /// Process spawn succeeded and returned one child pid.
    Launched { pid: u32 },
    /// Process spawn failed before a child could be started.
    Failed { error: String },
}

/// Historical record stored for one external command attempt.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandExecutionRecord {
    /// Frame on which the command attempt was recorded.
    pub frame: u64,
    /// Uptime snapshot captured when the command attempt was recorded.
    pub uptime_millis: u128,
    /// Human-readable origin such as a keybinding or startup action.
    pub origin: String,
    /// Resolved argv actually executed, if command resolution succeeded.
    pub command: Option<Vec<String>>,
    /// Candidate argv lists considered during command resolution.
    pub candidates: Vec<Vec<String>>,
    /// Final launch status once the command runner handled the request.
    pub status: Option<CommandExecutionStatus>,
}

/// Bounded history of recently attempted external commands.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandHistoryState {
    /// Maximum number of records retained in the in-memory ring buffer.
    pub limit: usize,
    /// Oldest-to-newest history of recent command attempts.
    pub items: Vec<CommandExecutionRecord>,
}

impl Default for CommandHistoryState {
    fn default() -> Self {
        Self { limit: DEFAULT_COMMAND_HISTORY_LIMIT, items: Vec::new() }
    }
}

impl CommandHistoryState {
    /// Appends one history record while enforcing the configured ring-buffer size.
    pub fn push(&mut self, record: CommandExecutionRecord) {
        if self.limit == 0 {
            self.items.clear();
            return;
        }

        self.items.push(record);
        if self.items.len() > self.limit {
            let overflow = self.items.len() - self.limit;
            self.items.drain(..overflow);
        }
    }
}
