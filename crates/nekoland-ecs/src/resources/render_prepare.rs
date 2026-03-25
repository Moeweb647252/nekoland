use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{
    MaterialParamsId, PlatformDmabufFormat, PlatformSurfaceBufferSource,
    PlatformSurfaceImportStrategy, PlatformSurfaceKind, ProcessShaderKey, QuadContent,
    RenderBindGroupLayoutKey, RenderItemId, RenderMaterialDescriptor, RenderMaterialId,
    RenderMaterialParamBlock, RenderRect, RenderTargetId, RenderTargetKind,
};

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceBufferAttachmentSnapshot {
    pub surfaces: BTreeMap<u64, SurfaceBufferAttachmentState>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceBufferAttachmentState {
    pub attached: bool,
    pub scale: i32,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderTargetAllocationPlan {
    pub outputs: BTreeMap<OutputId, OutputTargetAllocationPlan>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputTargetAllocationPlan {
    pub targets: BTreeMap<RenderTargetId, RenderTargetAllocationSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderTargetAllocationSpec {
    pub kind: RenderTargetKind,
    pub width: u32,
    pub height: u32,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceTextureBridgePlan {
    pub surfaces: BTreeMap<u64, SurfaceTextureImportDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceTextureImportDescriptor {
    pub surface_id: u64,
    pub surface_kind: PlatformSurfaceKind,
    pub buffer_source: PlatformSurfaceBufferSource,
    pub dmabuf_format: Option<PlatformDmabufFormat>,
    pub import_strategy: PlatformSurfaceImportStrategy,
    pub target_outputs: BTreeSet<OutputId>,
    pub content_version: u64,
    pub attached: bool,
    pub scale: i32,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PreparedSceneResources {
    pub outputs: BTreeMap<OutputId, OutputPreparedSceneResources>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputPreparedSceneResources {
    pub items: BTreeMap<RenderItemId, PreparedSceneItem>,
    pub ordered_items: Vec<RenderItemId>,
}

#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PreparedGpuResources {
    pub outputs: BTreeMap<OutputId, OutputPreparedGpuResources>,
    pub surface_imports: BTreeMap<u64, PreparedSurfaceImport>,
    pub material_bindings: BTreeMap<PreparedMaterialBindingKey, PreparedMaterialBinding>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputPreparedGpuResources {
    pub targets: BTreeMap<RenderTargetId, PreparedRenderTargetResource>,
    pub surface_imports: BTreeMap<u64, PreparedSurfaceImport>,
    pub process_shaders: BTreeSet<ProcessShaderKey>,
    pub material_bindings: BTreeSet<PreparedMaterialBindingKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedRenderTargetCacheKey {
    pub output_id: OutputId,
    pub target_id: RenderTargetId,
    pub kind: RenderTargetKind,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedRenderTargetResource {
    pub target_id: RenderTargetId,
    pub kind: RenderTargetKind,
    pub width: u32,
    pub height: u32,
    pub cache_key: PreparedRenderTargetCacheKey,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreparedSurfaceImportStrategy {
    ShmUpload,
    DmaBufImport,
    ExternalTextureImport,
    SinglePixelFill,
    #[default]
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedSurfaceImportCacheKey {
    pub surface_id: u64,
    pub content_version: u64,
    pub strategy: PreparedSurfaceImportStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedSurfaceImport {
    pub surface_id: u64,
    pub descriptor: SurfaceTextureImportDescriptor,
    pub strategy: PreparedSurfaceImportStrategy,
    pub cache_key: PreparedSurfaceImportCacheKey,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct PreparedMaterialBindingKey {
    pub material_id: RenderMaterialId,
    pub params_id: Option<MaterialParamsId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedMaterialBindingCacheKey {
    pub output_id: OutputId,
    pub binding_key: PreparedMaterialBindingKey,
    pub descriptor: RenderMaterialDescriptor,
    pub bind_group_layout: RenderBindGroupLayoutKey,
    pub params: Option<RenderMaterialParamBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedMaterialBinding {
    pub key: PreparedMaterialBindingKey,
    pub descriptor: RenderMaterialDescriptor,
    pub bind_group_layout: RenderBindGroupLayoutKey,
    pub params: Option<RenderMaterialParamBlock>,
    pub cache_key: PreparedMaterialBindingCacheKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PreparedSceneItem {
    Surface(PreparedSurfaceSceneItem),
    Quad(PreparedQuadSceneItem),
    Backdrop(PreparedBackdropSceneItem),
    CursorNamed(PreparedNamedCursorSceneItem),
    CursorSurface(PreparedSurfaceCursorSceneItem),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedSurfaceSceneItem {
    pub surface_id: u64,
    pub surface_kind: PlatformSurfaceKind,
    pub x: i32,
    pub y: i32,
    pub visible_rect: RenderRect,
    pub opacity: f32,
    pub import_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedQuadSceneItem {
    pub rect: RenderRect,
    pub visible_rect: RenderRect,
    pub content: QuadContent,
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedBackdropSceneItem {
    pub visible_rect: RenderRect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedNamedCursorSceneItem {
    pub icon_name: String,
    pub x: i32,
    pub y: i32,
    pub scale: u32,
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreparedSurfaceCursorSceneItem {
    pub surface_id: u64,
    pub x: i32,
    pub y: i32,
    pub visible_rect: RenderRect,
    pub opacity: f32,
    pub import_ready: bool,
}
