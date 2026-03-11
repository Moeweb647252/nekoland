use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceServerAction {
    Switch { workspace: String },
    Create { workspace: String },
    Destroy { workspace: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceServerRequest {
    pub action: WorkspaceServerAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWorkspaceServerRequests {
    pub items: Vec<WorkspaceServerRequest>,
}
