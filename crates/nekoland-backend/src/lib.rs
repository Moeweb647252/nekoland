//! Backend selection and runtime integrations for DRM, nested winit, and virtual output modes.

pub mod common;
pub mod components;
pub mod drm;
pub mod manager;
pub mod plugin;
pub mod traits;
pub mod virtual_output;
pub mod winit;

pub use common::outputs::BackendOutputRegistry;
pub use manager::BackendStatus;
pub use plugin::BackendPlugin;
pub use winit::backend::WinitWindowState;
