use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPresentationTimeline {
    pub output_name: String,
    pub refresh_interval_nanos: u64,
    pub present_time_nanos: u64,
    pub sequence: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPresentationEventRecord {
    pub output_name: String,
    pub refresh_interval_nanos: u64,
    pub present_time_nanos: u64,
    pub sequence: u64,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPresentationState {
    pub outputs: Vec<OutputPresentationTimeline>,
}

#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOutputPresentationEvents {
    pub items: Vec<OutputPresentationEventRecord>,
}
