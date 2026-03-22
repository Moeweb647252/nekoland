use crate::resources::PlatformSurfaceImportStrategy;
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

/// Import pipeline stage where a non-SHM surface failed to prepare or present.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformImportFailureStage {
    SurfaceImport,
    #[default]
    Present,
}

/// Structured diagnostic emitted when a backend advertises non-SHM import support but a concrete
/// import or present step still fails at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformImportDiagnostic {
    pub output_name: String,
    pub surface_id: Option<u64>,
    pub strategy: Option<PlatformSurfaceImportStrategy>,
    pub stage: PlatformImportFailureStage,
    pub message: String,
}

/// Recent non-SHM import diagnostics mirrored through `WaylandFeedback` for runtime inspection.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformImportDiagnosticsState {
    pub entries: Vec<PlatformImportDiagnostic>,
}

impl PlatformImportDiagnosticsState {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn push_surface_import_failure(
        &mut self,
        output_name: impl Into<String>,
        surface_id: u64,
        strategy: PlatformSurfaceImportStrategy,
        message: impl Into<String>,
    ) {
        self.push(PlatformImportDiagnostic {
            output_name: output_name.into(),
            surface_id: Some(surface_id),
            strategy: Some(strategy),
            stage: PlatformImportFailureStage::SurfaceImport,
            message: message.into(),
        });
    }

    pub fn push_present_failure(
        &mut self,
        output_name: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.push(PlatformImportDiagnostic {
            output_name: output_name.into(),
            surface_id: None,
            strategy: None,
            stage: PlatformImportFailureStage::Present,
            message: message.into(),
        });
    }

    fn push(&mut self, diagnostic: PlatformImportDiagnostic) {
        const MAX_ENTRIES: usize = 64;
        self.entries.push(diagnostic);
        if self.entries.len() > MAX_ENTRIES {
            let overflow = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(0..overflow);
        }
    }
}
