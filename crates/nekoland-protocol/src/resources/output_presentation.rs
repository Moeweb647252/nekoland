use bevy_ecs::prelude::Resource;
use nekoland_ecs::components::OutputId;
use nekoland_ecs::kinds::BackendEventQueue;
use serde::{Deserialize, Serialize};

/// Latest presentation timeline values known for one output.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPresentationTimeline {
    pub output_id: OutputId,
    pub refresh_interval_nanos: u64,
    pub present_time_nanos: u64,
    pub sequence: u64,
}

/// One presentation event emitted by a backend for a specific output.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPresentationEventRecord {
    pub output_id: OutputId,
    pub refresh_interval_nanos: u64,
    pub present_time_nanos: u64,
    pub sequence: u64,
}

/// Current presentation timeline snapshot across all outputs.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputPresentationState {
    pub outputs: Vec<OutputPresentationTimeline>,
}

/// Queue of presentation events waiting to be folded into `OutputPresentationState`.
pub type PendingOutputPresentationEvents = BackendEventQueue<OutputPresentationEventRecord>;
