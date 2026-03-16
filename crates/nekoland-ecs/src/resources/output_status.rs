use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Tracks the output names currently materialized in ECS by any backend runtime.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputRegistry {
    /// Output names currently known to be physically/backend connected.
    pub connected_outputs: Vec<String>,
    /// Output names currently materialized as enabled ECS entities.
    pub enabled_outputs: Vec<String>,
}
