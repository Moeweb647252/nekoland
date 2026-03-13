use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Monotonic compositor frame clock used by tests, history records, and snapshot generation.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositorClock {
    pub frame: u64,
    pub uptime_millis: u128,
}
