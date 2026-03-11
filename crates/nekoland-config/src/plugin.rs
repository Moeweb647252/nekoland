use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bevy_app::App;
use bevy_ecs::prelude::Resource;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_ecs::resources::CompositorConfig;

use crate::{hot_reload, loader, schema::NekolandConfigFile};

#[derive(Debug, Clone, Resource)]
pub struct LoadedConfigSource {
    pub path: PathBuf,
    pub loaded_from_disk: bool,
    pub last_observed_modified: Option<SystemTime>,
    pub successful_reloads: u64,
    pub last_reload_error: Option<String>,
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

        let (config, loaded_from_disk, successful_reloads, last_reload_error) =
            match loader::load_from_path(&self.path) {
                Ok(config) => {
                    tracing::info!(path = %self.path.display(), "loaded compositor config");
                    (config.into(), true, 1, None)
                }
                Err(error) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        %error,
                        "falling back to built-in default config"
                    );
                    (
                        CompositorConfig::from(NekolandConfigFile::default()),
                        false,
                        0,
                        Some(error.to_string()),
                    )
                }
            };

        app.insert_resource(config)
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
