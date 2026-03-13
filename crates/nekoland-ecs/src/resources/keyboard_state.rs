use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Current keyboard focus target tracked by shell/input systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardFocusState {
    pub focused_surface: Option<u64>,
}

/// Coarse modifier snapshot derived from backend key events.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}
