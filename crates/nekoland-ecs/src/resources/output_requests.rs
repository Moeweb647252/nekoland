use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputServerAction {
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputServerRequest {
    pub action: OutputServerAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOutputServerRequests {
    pub items: Vec<OutputServerRequest>,
}
