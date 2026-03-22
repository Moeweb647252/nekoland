//! Backend selection and runtime integrations for DRM, nested winit, and virtual output modes.

pub mod common;
pub mod components;
pub mod drm;
pub mod manager;
pub mod plugin;
pub mod traits;
pub mod virtual_output;
pub mod winit;

pub use manager::{BackendStatus, SharedBackendManager};
pub use plugin::{
    BackendPlugin, BackendWaylandSubAppPlugin, extract_backend_wayland_subapp_inputs,
};
pub use winit::backend::WinitWindowState;
