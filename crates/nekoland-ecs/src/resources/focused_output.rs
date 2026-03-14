use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Names the output currently targeted by pointer/keyboard-oriented default routing.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusedOutputState {
    pub name: Option<String>,
}
