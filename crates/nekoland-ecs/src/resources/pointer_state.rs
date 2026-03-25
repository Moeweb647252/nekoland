//! Pointer position and viewport-pan state shared across input and shell systems.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Global pointer position shared across input, focus, and virtual-output capture systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalPointerPosition {
    pub x: f64,
    pub y: f64,
}

/// Last physical pointer position reported by the backend before compositor-side routing.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PhysicalPointerPosition {
    pub x: f64,
    pub y: f64,
    pub initialized: bool,
    pub needs_resync: bool,
}

/// Per-frame raw pointer delta accumulated from backend motion events.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PointerDelta {
    pub dx: f64,
    pub dy: f64,
}

/// Tracks whether the pointer is currently driving interactive viewport panning.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewportPointerPanState {
    pub active: bool,
}
