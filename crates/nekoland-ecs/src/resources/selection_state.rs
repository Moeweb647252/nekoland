//! Clipboard, primary-selection, and drag-and-drop state mirrored into ECS.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::SeatId;

/// Normalized owner label for clipboard-like selections.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOwner {
    #[default]
    Client,
    Compositor,
}

/// Clipboard selection snapshot stored in ECS.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardSelection {
    pub seat_id: SeatId,
    pub mime_types: Vec<String>,
    pub owner: SelectionOwner,
    pub persisted_mime_types: Vec<String>,
}

/// Current clipboard selection state.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipboardSelectionState {
    pub selection: Option<ClipboardSelection>,
}

/// Primary-selection snapshot stored in ECS.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelection {
    pub seat_id: SeatId,
    pub mime_types: Vec<String>,
    pub owner: SelectionOwner,
    pub persisted_mime_types: Vec<String>,
}

/// Current primary-selection state.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimarySelectionState {
    pub selection: Option<PrimarySelection>,
}

/// Drag-and-drop state exported from protocol processing into ECS.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DragAndDropState {
    pub active_session: Option<DragAndDropSession>,
    pub last_drop: Option<DragAndDropDrop>,
}

/// Active drag session metadata.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DragAndDropSession {
    pub seat_id: SeatId,
    pub source_surface_id: Option<u64>,
    pub icon_surface_id: Option<u64>,
    pub mime_types: Vec<String>,
    pub accepted_mime_type: Option<String>,
    pub chosen_action: Option<String>,
}

/// Most recently observed drop result.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DragAndDropDrop {
    pub seat_id: SeatId,
    pub source_surface_id: Option<u64>,
    pub target_surface_id: Option<u64>,
    pub validated: bool,
    pub mime_types: Vec<String>,
}
