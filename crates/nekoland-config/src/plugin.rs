use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bevy_app::App;
use bevy_ecs::prelude::Resource;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_ecs::resources::{CompositorConfig, KeyboardLayoutState};

use crate::{hot_reload, loader, schema::NekolandConfigFile};

/// Tracks where the active config came from and what happened during hot reload attempts.
#[derive(Debug, Clone, Resource)]
pub struct LoadedConfigSource {
    pub path: PathBuf,
    pub loaded_from_disk: bool,
    pub last_observed_modified: Option<SystemTime>,
    pub successful_reloads: u64,
    pub last_reload_error: Option<String>,
}

/// External request flag used to force one config reload on the next extract tick.
#[derive(Debug, Clone, Default, Resource)]
pub struct ConfigReloadRequest {
    pub requested: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigPlugin {
    path: PathBuf,
}

impl ConfigPlugin {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for ConfigPlugin {
    fn default() -> Self {
        Self::new("config/default.toml")
    }
}

impl NekolandPlugin for ConfigPlugin {
    fn build(&self, app: &mut App) {
        let last_observed_modified = hot_reload::observed_modified_at(&self.path);
        hot_reload::install_config_watch_source(app, self.path.clone(), last_observed_modified);
        let default_config = match CompositorConfig::try_from(NekolandConfigFile::default()) {
            Ok(config) => config,
            Err(error) => {
                tracing::error!(
                    %error,
                    "built-in default config failed to normalize; falling back to CompositorConfig::default()"
                );
                CompositorConfig::default()
            }
        };

        // Falling back to defaults keeps the compositor bootable even when the configured path is
        // missing or malformed; the failure is still preserved in `LoadedConfigSource`.
        let (config, loaded_from_disk, successful_reloads, last_reload_error) =
            match loader::load_from_path(&self.path) {
                Ok(config) => match CompositorConfig::try_from(config) {
                    Ok(config) => {
                        tracing::info!(path = %self.path.display(), "loaded compositor config");
                        (config, true, 1, None)
                    }
                    Err(error) => {
                        tracing::warn!(
                            path = %self.path.display(),
                            %error,
                            "falling back to built-in default config"
                        );
                        (default_config.clone(), false, 0, Some(error))
                    }
                },
                Err(error) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        %error,
                        "falling back to built-in default config"
                    );
                    (default_config, false, 0, Some(error.to_string()))
                }
            };

        let keyboard_layout_state =
            KeyboardLayoutState::from_config(&config.keyboard_layouts, &config.current_keyboard_layout);

        app.insert_resource(config)
            .insert_resource(keyboard_layout_state)
            .insert_resource(ConfigReloadRequest::default())
            .insert_resource(LoadedConfigSource {
                path: self.path.clone(),
                loaded_from_disk,
                last_observed_modified,
                successful_reloads,
                last_reload_error,
            })
            .add_systems(nekoland_core::schedules::ExtractSchedule, hot_reload::hot_reload_system);
    }
}
