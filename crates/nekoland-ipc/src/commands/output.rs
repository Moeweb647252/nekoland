use serde::{Deserialize, Serialize};

/// Mutable output-management commands accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputCommand {
    Configure {
        output: String,
        mode: String,
        #[serde(default)]
        scale: Option<u32>,
    },
    Enable {
        output: String,
    },
    Disable {
        output: String,
    },
}
