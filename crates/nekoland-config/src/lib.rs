//! Config-file schema, loading, normalization, and hot-reload support.
#![warn(missing_docs)]

/// Action-list parsing and normalization shared by startup commands and related config surfaces.
pub mod action_config;
/// File watching and extract-phase hot-reload entrypoints.
pub mod hot_reload;
/// Shortcut override schema helpers for user-facing config files.
pub mod keybind_config;
/// Disk loading helpers for TOML and RON config sources.
pub mod loader;
/// Plugin entrypoint and runtime resources for config loading and reload state.
pub mod plugin;
/// Normalized runtime resources derived from the on-disk config schema.
pub mod resources;
/// TOML-facing schema loaded from disk before normalization.
pub mod schema;
/// Theme-related schema types exposed in the config file.
pub mod theme;

pub use plugin::{ConfigPlugin, ConfigReloadRequest, LoadedConfigSource};
pub use schema::NekolandConfigFile;
