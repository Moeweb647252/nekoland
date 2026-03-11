use crate::resources::DEFAULT_COMMAND_HISTORY_LIMIT;
use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CommandExecutionStatus {
    Launched { pid: u32 },
    Failed { error: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandExecutionRecord {
    pub frame: u64,
    pub uptime_millis: u128,
    pub origin: String,
    pub command: Option<Vec<String>>,
    pub candidates: Vec<Vec<String>>,
    pub status: Option<CommandExecutionStatus>,
}

#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandHistoryState {
    pub limit: usize,
    pub items: Vec<CommandExecutionRecord>,
}

impl Default for CommandHistoryState {
    fn default() -> Self {
        Self { limit: DEFAULT_COMMAND_HISTORY_LIMIT, items: Vec::new() }
    }
}

impl CommandHistoryState {
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
