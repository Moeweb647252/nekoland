//! Present-time audit snapshots captured by backend execution for debugging and IPC.

#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;

/// Normalized element kinds captured by backend-side present audits.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PresentAuditElementKind {
    Window,
    Popup,
    Layer,
    Quad,
    Backdrop,
    Compositor,
    Cursor,
    #[default]
    Unknown,
}

/// Output-local scene element snapshot captured from present-time inputs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PresentAuditElement {
    pub surface_id: u64,
    pub kind: PresentAuditElementKind,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z_index: i32,
    pub opacity: f32,
}

/// Latest audit snapshot for one output during backend present.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputPresentAudit {
    pub output_name: String,
    pub frame: u64,
    pub uptime_millis: u64,
    pub elements: Vec<PresentAuditElement>,
}

/// Latest backend-generic present audit snapshots keyed by runtime output id.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PresentAuditState {
    pub outputs: BTreeMap<OutputId, OutputPresentAudit>,
}
