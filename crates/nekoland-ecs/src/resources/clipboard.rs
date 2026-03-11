use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOwner {
    #[default]
    Client,
    Compositor,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardSelection {
    pub seat_name: String,
    pub mime_types: Vec<String>,
    pub owner: SelectionOwner,
    pub persisted_mime_types: Vec<String>,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardSelectionState {
    pub selection: Option<ClipboardSelection>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelection {
    pub seat_name: String,
    pub mime_types: Vec<String>,
    pub owner: SelectionOwner,
    pub persisted_mime_types: Vec<String>,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelectionState {
    pub selection: Option<PrimarySelection>,
}
