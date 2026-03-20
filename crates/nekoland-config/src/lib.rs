//! Config-file schema, loading, normalization, and hot-reload support.

pub mod action_config;
pub mod hot_reload;
pub mod keybind_config;
pub mod loader;
pub mod plugin;
pub mod resources;
pub mod schema;
pub mod theme;

pub use plugin::{ConfigPlugin, ConfigReloadRequest, LoadedConfigSource};
pub use schema::NekolandConfigFile;
