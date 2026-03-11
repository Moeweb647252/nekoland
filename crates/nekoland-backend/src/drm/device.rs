use bevy_ecs::prelude::ResMut;

use crate::traits::{Backend, BackendKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrmDevice {
    pub path: String,
}

impl Backend for DrmDevice {
    fn kind(&self) -> BackendKind {
        BackendKind::Drm
    }

    fn label(&self) -> &str {
        "drm"
    }
}

pub fn drm_device_system(mut selected_backend: ResMut<crate::traits::SelectedBackend>) {
    if selected_backend.kind == BackendKind::Drm {
        selected_backend.description = "/dev/dri/card0".to_owned();
    }
}
