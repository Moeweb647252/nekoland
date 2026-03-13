use serde::{Deserialize, Serialize};

/// Workspace-management commands accepted by the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceCommand {
    Switch { workspace: String },
    Create { workspace: String },
    Destroy { workspace: String },
}
