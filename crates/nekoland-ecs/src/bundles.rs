//! Canonical bundle groupings used when materializing common compositor entities.
//!
//! The bundle type describes the intent; individual field names already mirror the contained
//! component types, so field-level rustdoc would add little beyond noise here.
#![allow(missing_docs)]

use bevy_ecs::bundle::Bundle;

use crate::components::{
    BorderTheme, BufferState, DesiredOutputName, LayerAnchor, LayerLevel, LayerMargins,
    LayerShellSurface, OutputDevice, OutputPlacement, OutputProperties, OutputViewport,
    OutputWorkArea, ServerDecoration, SurfaceContentVersion, SurfaceGeometry, Window,
    WindowAnimation, WindowLayout, WindowManagementHints, WindowMode, WindowSceneGeometry,
    WindowViewportVisibility, WlSurfaceHandle, X11Window,
};

/// Canonical component set for a native XDG toplevel window entity.
#[derive(Bundle, Clone, Debug, Default)]
pub struct WindowBundle {
    pub surface: WlSurfaceHandle,
    pub geometry: SurfaceGeometry,
    pub scene_geometry: WindowSceneGeometry,
    pub viewport_visibility: WindowViewportVisibility,
    pub buffer: BufferState,
    pub content_version: SurfaceContentVersion,
    pub window: Window,
    pub management_hints: WindowManagementHints,
    pub layout: WindowLayout,
    pub mode: WindowMode,
    pub decoration: ServerDecoration,
    pub border_theme: BorderTheme,
    pub animation: WindowAnimation,
}

/// Canonical component set for an XWayland-managed window entity.
#[derive(Bundle, Clone, Debug, Default)]
pub struct X11WindowBundle {
    pub surface: WlSurfaceHandle,
    pub geometry: SurfaceGeometry,
    pub scene_geometry: WindowSceneGeometry,
    pub viewport_visibility: WindowViewportVisibility,
    pub buffer: BufferState,
    pub content_version: SurfaceContentVersion,
    pub window: Window,
    pub management_hints: WindowManagementHints,
    pub x11_window: X11Window,
    pub layout: WindowLayout,
    pub mode: WindowMode,
    pub decoration: ServerDecoration,
    pub border_theme: BorderTheme,
    pub animation: WindowAnimation,
}

/// Canonical component set for one output entity.
#[derive(Bundle, Clone, Debug, Default)]
pub struct OutputBundle {
    pub output: OutputDevice,
    pub properties: OutputProperties,
    pub placement: OutputPlacement,
    pub viewport: OutputViewport,
    pub work_area: OutputWorkArea,
}

/// Canonical component set for one layer-shell surface entity.
#[derive(Bundle, Clone, Debug, Default)]
pub struct LayerSurfaceBundle {
    pub surface: WlSurfaceHandle,
    pub geometry: SurfaceGeometry,
    pub buffer: BufferState,
    pub content_version: SurfaceContentVersion,
    pub layer_surface: LayerShellSurface,
    pub desired_output_name: DesiredOutputName,
    pub anchor: LayerAnchor,
    pub animation: WindowAnimation,
}

/// Input data used to build a [`LayerSurfaceBundle`] directly from protocol lifecycle events.
#[derive(Clone, Debug)]
pub struct LayerSurfaceBundleSpec {
    pub surface_id: u64,
    pub namespace: String,
    pub output: Option<String>,
    pub layer: LayerLevel,
    pub anchor: LayerAnchor,
    pub desired_width: u32,
    pub desired_height: u32,
    pub exclusive_zone: i32,
    pub margins: LayerMargins,
}

impl LayerSurfaceBundle {
    /// Builds a layer bundle directly from the protocol create request payload.
    pub fn new(spec: LayerSurfaceBundleSpec) -> Self {
        let LayerSurfaceBundleSpec {
            surface_id,
            namespace,
            output,
            layer,
            anchor,
            desired_width,
            desired_height,
            exclusive_zone,
            margins,
        } = spec;
        Self {
            surface: WlSurfaceHandle { id: surface_id },
            geometry: SurfaceGeometry {
                x: 0,
                y: 0,
                width: desired_width.max(1),
                height: desired_height.max(1),
            },
            buffer: BufferState { attached: false, scale: 1 },
            content_version: SurfaceContentVersion::default(),
            layer_surface: LayerShellSurface {
                namespace,
                layer,
                desired_width,
                desired_height,
                exclusive_zone,
                margins,
            },
            desired_output_name: DesiredOutputName(output),
            anchor,
            animation: WindowAnimation::default(),
        }
    }
}
