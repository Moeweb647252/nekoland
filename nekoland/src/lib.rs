use std::env;
use std::path::PathBuf;

use nekoland_backend::BackendPlugin;
use nekoland_config::ConfigPlugin;
use nekoland_core::prelude::NekolandApp;
use nekoland_input::InputPlugin;
use nekoland_ipc::IpcPlugin;
use nekoland_protocol::ProtocolPlugin;
use nekoland_render::RenderPlugin;
use nekoland_shell::ShellPlugin;

/// Resolves the default config path, allowing `NEKOLAND_CONFIG` to override the repository default.
pub fn default_config_path() -> PathBuf {
    env::var_os("NEKOLAND_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/default.toml"))
}

/// Builds the standard compositor application by wiring every crate plugin into the core app.
pub fn build_app(config_path: impl Into<PathBuf>) -> NekolandApp {
    let mut app = NekolandApp::new("nekoland");
    app.add_plugin(ConfigPlugin::new(config_path.into()))
        .add_plugin(ProtocolPlugin)
        .add_plugin(BackendPlugin)
        .add_plugin(InputPlugin)
        .add_plugin(ShellPlugin)
        .add_plugin(RenderPlugin)
        .add_plugin(IpcPlugin);
    app
}

/// Convenience wrapper that builds the app using the default config path resolution rules.
pub fn build_default_app() -> NekolandApp {
    build_app(default_config_path())
}
