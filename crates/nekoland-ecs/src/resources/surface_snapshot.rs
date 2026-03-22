use std::collections::BTreeMap;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Stable platform-facing surface kinds exported without live protocol handles.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSurfaceKind {
    Toplevel,
    Popup,
    Layer,
    Cursor,
    #[default]
    Unknown,
}

/// Stable platform-facing buffer source kinds exported without live protocol handles.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSurfaceBufferSource {
    Shm,
    DmaBuf,
    SinglePixel,
    #[default]
    Unknown,
}

/// Stable platform-owned import strategy exported without exposing backend/protocol internals.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSurfaceImportStrategy {
    ShmUpload,
    DmaBufImport,
    ExternalTextureImport,
    SinglePixelFill,
    #[default]
    Unsupported,
}

/// Stable dma-buf format metadata exported without exposing smithay/drm-fourcc types directly.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlatformDmabufFormat {
    pub code: u32,
    pub modifier: u64,
}

/// Immutable snapshot for one compositor-managed platform surface.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformSurfaceSnapshot {
    pub surface_id: u64,
    pub kind: PlatformSurfaceKind,
    pub buffer_source: PlatformSurfaceBufferSource,
    pub dmabuf_format: Option<PlatformDmabufFormat>,
    pub import_strategy: PlatformSurfaceImportStrategy,
    pub attached: bool,
    pub scale: i32,
    pub content_version: u64,
}

/// Frame-visible surface registry snapshot safe to share across app boundaries.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlatformSurfaceSnapshotState {
    pub surfaces: BTreeMap<u64, PlatformSurfaceSnapshot>,
}

impl PlatformSurfaceSnapshotState {
    pub fn kind(&self, surface_id: u64) -> PlatformSurfaceKind {
        self.surfaces.get(&surface_id).map(|snapshot| snapshot.kind).unwrap_or_default()
    }

    pub fn buffer_source(&self, surface_id: u64) -> PlatformSurfaceBufferSource {
        self.surfaces.get(&surface_id).map(|snapshot| snapshot.buffer_source).unwrap_or_default()
    }

    pub fn dmabuf_format(&self, surface_id: u64) -> Option<PlatformDmabufFormat> {
        self.surfaces.get(&surface_id).and_then(|snapshot| snapshot.dmabuf_format)
    }

    pub fn attached(&self, surface_id: u64) -> bool {
        self.surfaces.get(&surface_id).is_some_and(|snapshot| snapshot.attached)
    }

    pub fn import_strategy(&self, surface_id: u64) -> PlatformSurfaceImportStrategy {
        self.surfaces.get(&surface_id).map(|snapshot| snapshot.import_strategy).unwrap_or_default()
    }

    pub fn scale(&self, surface_id: u64) -> i32 {
        self.surfaces.get(&surface_id).map(|snapshot| snapshot.scale).unwrap_or(1)
    }

    pub fn content_version(&self, surface_id: u64) -> u64 {
        self.surfaces.get(&surface_id).map(|snapshot| snapshot.content_version).unwrap_or_default()
    }
}
