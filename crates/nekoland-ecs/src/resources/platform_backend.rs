use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Backend families exposed through platform-facing snapshots.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlatformBackendKind {
    Drm,
    Winit,
    Virtual,
    #[default]
    Auto,
}

/// Runtime role exposed for one backend instance in platform-facing snapshots.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlatformBackendRole {
    PrimaryDisplay,
    SecondaryDisplay,
    CaptureSink,
    DebugSink,
}

/// Human-readable descriptor for one backend runtime instance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformBackendDescriptor {
    pub id: u64,
    pub kind: PlatformBackendKind,
    pub role: PlatformBackendRole,
    pub label: String,
    pub description: String,
}

/// Platform-facing snapshot of the currently active backend runtimes.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformBackendState {
    pub active: Vec<PlatformBackendDescriptor>,
}

impl PlatformBackendState {
    pub fn primary_display(&self) -> Option<&PlatformBackendDescriptor> {
        self.active.iter().find(|descriptor| {
            descriptor.role == PlatformBackendRole::PrimaryDisplay
                || descriptor.role == PlatformBackendRole::SecondaryDisplay
        })
    }
}

/// Platform-facing import capabilities exported across app boundaries.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformImportCapabilities {
    pub dmabuf_importable: bool,
}
