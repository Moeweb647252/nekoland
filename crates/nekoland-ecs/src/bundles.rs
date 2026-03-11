use bevy_ecs::bundle::Bundle;

use crate::components::{
    BorderTheme, BufferState, LayerAnchor, LayerLevel, LayerMargins, LayerShellSurface,
    OutputDevice, OutputProperties, ServerDecoration, SurfaceGeometry, WindowAnimation,
    WindowState, WlSurfaceHandle, X11Window, XdgWindow,
};

#[derive(Bundle, Clone, Debug, Default)]
pub struct WindowBundle {
    pub surface: WlSurfaceHandle,
    pub geometry: SurfaceGeometry,
    pub buffer: BufferState,
    pub window: XdgWindow,
    pub state: WindowState,
    pub decoration: ServerDecoration,
    pub border_theme: BorderTheme,
    pub animation: WindowAnimation,
}

#[derive(Bundle, Clone, Debug, Default)]
pub struct X11WindowBundle {
    pub surface: WlSurfaceHandle,
    pub geometry: SurfaceGeometry,
    pub buffer: BufferState,
    pub window: XdgWindow,
    pub x11_window: X11Window,
    pub state: WindowState,
    pub decoration: ServerDecoration,
    pub border_theme: BorderTheme,
    pub animation: WindowAnimation,
}

#[derive(Bundle, Clone, Debug, Default)]
pub struct OutputBundle {
    pub output: OutputDevice,
    pub properties: OutputProperties,
}

#[derive(Bundle, Clone, Debug, Default)]
pub struct LayerSurfaceBundle {
    pub surface: WlSurfaceHandle,
    pub geometry: SurfaceGeometry,
    pub buffer: BufferState,
    pub layer_surface: LayerShellSurface,
    pub anchor: LayerAnchor,
    pub animation: WindowAnimation,
}

impl LayerSurfaceBundle {
    pub fn new(
        surface_id: u64,
        namespace: String,
        output: Option<String>,
        layer: LayerLevel,
        anchor: LayerAnchor,
        desired_width: u32,
        desired_height: u32,
        exclusive_zone: i32,
        margins: LayerMargins,
    ) -> Self {
        Self {
            surface: WlSurfaceHandle { id: surface_id },
            geometry: SurfaceGeometry {
                x: 0,
                y: 0,
                width: desired_width.max(1),
                height: desired_height.max(1),
            },
            buffer: BufferState { attached: false, scale: 1 },
            layer_surface: LayerShellSurface {
                namespace,
                output,
                layer,
                desired_width,
                desired_height,
                exclusive_zone,
                margins,
            },
            anchor,
            animation: WindowAnimation::default(),
        }
    }
}
