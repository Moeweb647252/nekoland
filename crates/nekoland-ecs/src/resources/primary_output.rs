use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Names the compositor's current primary output for layout decisions that should not silently
/// depend on query iteration order.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrimaryOutputState {
    pub name: Option<String>,
}
