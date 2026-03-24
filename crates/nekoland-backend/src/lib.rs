//! Backend selection and runtime integrations for DRM, nested winit, and virtual output modes.

pub mod common;
pub mod components;
pub mod drm;
pub mod manager;
pub mod plugin;
pub mod traits;
pub mod virtual_output;
pub mod winit;

pub use manager::{BackendStatus, SharedBackendManager, set_requested_backend_override};
pub use plugin::extract::extract_backend_wayland_subapp_inputs;
pub use plugin::{BackendPlugin, BackendWaylandSubAppPlugin};
pub use winit::backend::WinitWindowState;
