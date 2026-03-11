use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::SurfaceExtent;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct X11WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<(i32, i32, SurfaceExtent)> for X11WindowGeometry {
    fn from((x, y, size): (i32, i32, SurfaceExtent)) -> Self {
        Self { x, y, width: size.width, height: size.height }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum X11LifecycleAction {
    Mapped {
        window_id: u32,
        override_redirect: bool,
        title: String,
        app_id: String,
        geometry: X11WindowGeometry,
    },
    Reconfigured {
        title: String,
        app_id: String,
        geometry: X11WindowGeometry,
    },
    Maximize,
    UnMaximize,
    Fullscreen,
    UnFullscreen,
    Minimize,
    UnMinimize,
    InteractiveMove {
        button: u32,
    },
    InteractiveResize {
        button: u32,
        edges: String,
    },
    Unmapped,
    Destroyed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct X11LifecycleRequest {
    pub surface_id: u64,
    pub action: X11LifecycleAction,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingX11Requests {
    pub items: Vec<X11LifecycleRequest>,
}
