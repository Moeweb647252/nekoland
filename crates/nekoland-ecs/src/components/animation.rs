use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WindowAnimation {
    pub progress: f32,
    pub fade: FadeState,
}

#[derive(Component, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FadeState {
    In,
    Out,
    #[default]
    Idle,
}
