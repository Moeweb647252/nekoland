use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum XdgSurfaceRole {
    #[default]
    Toplevel,
    Popup,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceExtent {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub reposition_token: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowLifecycleAction {
    Committed { role: XdgSurfaceRole, size: Option<SurfaceExtent> },
    ConfigureRequested { role: XdgSurfaceRole },
    AckConfigure { role: XdgSurfaceRole, serial: u32 },
    MetadataChanged { title: Option<String>, app_id: Option<String> },
    InteractiveMove { seat_name: String, serial: u32 },
    InteractiveResize { seat_name: String, serial: u32, edges: String },
    Maximize,
    UnMaximize,
    Fullscreen { output_name: Option<String> },
    UnFullscreen,
    Minimize,
    PopupCreated { parent_surface_id: Option<u64>, placement: PopupPlacement },
    PopupRepositioned { placement: PopupPlacement },
    PopupGrab { seat_name: String, serial: u32 },
    Destroyed { role: XdgSurfaceRole },
}

impl Default for WindowLifecycleAction {
    fn default() -> Self {
        Self::Committed { role: XdgSurfaceRole::Toplevel, size: None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowLifecycleRequest {
    pub surface_id: u64,
    pub action: WindowLifecycleAction,
}

impl Default for WindowLifecycleRequest {
    fn default() -> Self {
        Self { surface_id: 0, action: WindowLifecycleAction::default() }
    }
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingXdgRequests {
    pub items: Vec<WindowLifecycleRequest>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputEventRecord {
    pub source: String,
    pub detail: String,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingInputEvents {
    pub items: Vec<InputEventRecord>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputEventRecord {
    pub output_name: String,
    pub change: String,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOutputEvents {
    pub items: Vec<OutputEventRecord>,
}
