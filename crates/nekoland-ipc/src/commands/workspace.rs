use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceCommand {
    Switch { workspace: String },
    Create { workspace: String },
    Destroy { workspace: String },
}
