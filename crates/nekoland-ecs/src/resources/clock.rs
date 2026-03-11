use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositorClock {
    pub frame: u64,
    pub uptime_millis: u128,
}
