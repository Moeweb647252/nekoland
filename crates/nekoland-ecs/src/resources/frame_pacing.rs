use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FramePacingState {
    pub frame_callbacks_sent: u64,
    pub presentation_batches: u64,
    pub callback_surface_ids: Vec<u64>,
    pub throttled_surface_ids: Vec<u64>,
    pub presentation_surface_ids: Vec<u64>,
}
