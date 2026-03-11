use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackendKind {
    Drm,
    Winit,
    Virtual,
    #[default]
    Auto,
}

pub trait Backend {
    fn kind(&self) -> BackendKind;
    fn label(&self) -> &str;
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectedBackend {
    pub kind: BackendKind,
    pub description: String,
}
