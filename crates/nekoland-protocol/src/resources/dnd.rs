use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Drag-and-drop state exported from protocol processing into ECS.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DragAndDropState {
    pub active_session: Option<DragAndDropSession>,
    pub last_drop: Option<DragAndDropDrop>,
}

/// Active drag session metadata.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DragAndDropSession {
    pub seat_name: String,
    pub source_surface_id: Option<u64>,
    pub icon_surface_id: Option<u64>,
    pub mime_types: Vec<String>,
    pub accepted_mime_type: Option<String>,
    pub chosen_action: Option<String>,
}

/// Most recently observed drop result.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DragAndDropDrop {
    pub seat_name: String,
    pub source_surface_id: Option<u64>,
    pub target_surface_id: Option<u64>,
    pub validated: bool,
    pub mime_types: Vec<String>,
}
