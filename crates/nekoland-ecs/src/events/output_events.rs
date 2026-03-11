use bevy_ecs::prelude::Message;
use serde::{Deserialize, Serialize};

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputConnected {
    pub name: String,
}

#[derive(Message, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputDisconnected {
    pub name: String,
}
