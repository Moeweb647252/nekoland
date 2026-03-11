use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCommandRequest {
    pub origin: String,
    pub candidates: Vec<Vec<String>>,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingExternalCommandRequests {
    pub items: Vec<ExternalCommandRequest>,
}
