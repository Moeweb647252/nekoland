use bevy_ecs::prelude::Message;
use serde::{Deserialize, Serialize};

/// Notification that an output became available in ECS.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputConnected {
    pub name: String,
}

/// Notification that an output was removed from ECS.
#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputDisconnected {
    pub name: String,
}
