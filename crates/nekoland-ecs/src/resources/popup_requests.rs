use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PopupServerAction {
    Dismiss,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupServerRequest {
    pub surface_id: u64,
    pub action: PopupServerAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingPopupServerRequests {
    pub items: Vec<PopupServerRequest>,
}
