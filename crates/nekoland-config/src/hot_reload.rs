use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use bevy_app::App;
use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{NonSendMut, ResMut};
use calloop::generic::Generic;
use calloop::{Interest, Mode, PostAction};
use nekoland_core::calloop::CalloopSourceRegistry;
use nekoland_core::error::NekolandError;
use nekoland_ecs::resources::{CompositorConfig, KeyboardLayoutState};
use nix::errno::Errno;
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

use crate::{
    loader,
    plugin::{ConfigReloadRequest, LoadedConfigSource},
};

/// Bridges file-system notifications into ECS so config reload stays a normal scheduled system
/// instead of mutating resources inside the watcher callback.
#[derive(Debug, Clone)]
pub struct ConfigHotReloadSource {
    shared: Rc<RefCell<ConfigHotReloadShared>>,
}

#[derive(Debug, Clone, Copy)]
struct ConfigHotReloadShared {
    watcher_active: bool,
    last_observed_modified: Option<SystemTime>,
    pending_change: bool,
}

impl ConfigHotReloadSource {
    fn new(last_observed_modified: Option<SystemTime>) -> Self {
        Self {
            shared: Rc::new(RefCell::new(ConfigHotReloadShared {
                watcher_active: false,
                last_observed_modified,
                pending_change: false,
            })),
        }
    }

    fn take_pending_modified(&self) -> Option<Option<SystemTime>> {
        let mut shared = self.shared.borrow_mut();
        if !shared.pending_change {
            return None;
        }

        shared.pending_change = false;
        Some(shared.last_observed_modified)
    }

    fn sync_observed_modified(&self, observed_modified: Option<SystemTime>) {
        let mut shared = self.shared.borrow_mut();
        shared.last_observed_modified = observed_modified;
    }

    fn watcher_active(&self) -> bool {
        self.shared.borrow().watcher_active
    }
}

pub(crate) fn observed_modified_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok())
}

pub(crate) fn install_config_watch_source(
    app: &mut App,
    path: PathBuf,
    last_observed_modified: Option<SystemTime>,
) {
    if app.world().get_non_send_resource::<ConfigHotReloadSource>().is_some() {
        return;
    }

    if app.world().get_non_send_resource::<CalloopSourceRegistry>().is_none() {
        app.insert_non_send_resource(CalloopSourceRegistry::default());
    }

    let source = ConfigHotReloadSource::new(last_observed_modified);
    let shared = source.shared.clone();
    let Some(mut registry) = app.world_mut().get_non_send_resource_mut::<CalloopSourceRegistry>()
    else {
        tracing::warn!(
            path = %path.display(),
            "config hot reload registry was unavailable; watcher install skipped"
        );
        app.insert_non_send_resource(source);
        return;
    };

    registry.push(move |handle| install_linux_inotify_source(handle, path.clone(), shared.clone()));

    app.insert_non_send_resource(source);
}

/// Reloads the config only after the watcher or polling fallback observes a file timestamp change.
/// On parse failure the previous config stays live and the error is surfaced through Bevy's
/// error channel.
pub fn hot_reload_system(
    mut config: ResMut<CompositorConfig>,
    mut keyboard_layout_state: ResMut<KeyboardLayoutState>,
    mut config_source: ResMut<LoadedConfigSource>,
    mut reload_request: ResMut<ConfigReloadRequest>,
    reload_source: NonSendMut<ConfigHotReloadSource>,
) -> BevyResult {
    let force_reload = std::mem::take(&mut reload_request.requested);
    let current_modified = if reload_source.watcher_active() && !force_reload {
        let Some(current_modified) = reload_source.take_pending_modified() else {
            tracing::trace!(
                path = %config_source.path.display(),
                loaded_from_disk = config_source.loaded_from_disk,
                successful_reloads = config_source.successful_reloads,
                "config hot reload tick"
            );
            return Ok(());
        };
        current_modified
    } else {
        // Tests and unsupported environments can run without an inotify source, so keep a
        // metadata-based fallback for correctness.
        let current_modified = observed_modified_at(&config_source.path);
        if !force_reload && current_modified == config_source.last_observed_modified {
            tracing::trace!(
                path = %config_source.path.display(),
                loaded_from_disk = config_source.loaded_from_disk,
                successful_reloads = config_source.successful_reloads,
                "config hot reload tick"
            );
            return Ok(());
        }

        reload_source.sync_observed_modified(current_modified);
        current_modified
    };

    config_source.last_observed_modified = current_modified;

    match loader::load_from_path(&config_source.path) {
        Ok(next_config) => {
            let next_config = CompositorConfig::try_from(next_config).map_err(|error| {
                NekolandError::Config(format!(
                    "keeping previous compositor config after reload failure for {}: {error}",
                    config_source.path.display()
                ))
            })?;
            let previous_layout_name = keyboard_layout_state.active_name().to_owned();
            keyboard_layout_state.apply_layouts(
                &next_config.keyboard_layouts,
                Some(next_config.current_keyboard_layout.as_str()),
                Some(previous_layout_name.as_str()),
            );
            *config = next_config;
            config_source.loaded_from_disk = true;
            config_source.successful_reloads += 1;
            config_source.last_reload_error = None;

            tracing::info!(
                path = %config_source.path.display(),
                successful_reloads = config_source.successful_reloads,
                "reloaded compositor config from disk"
            );
            Ok(())
        }
        Err(error) => {
            let error = error.to_string();
            config_source.last_reload_error = Some(error.clone());

            Err(NekolandError::Config(format!(
                "keeping previous compositor config after reload failure for {}: {error}",
                config_source.path.display()
            ))
            .into())
        }
    }
}

fn install_linux_inotify_source(
    handle: &calloop::LoopHandle<'_, ()>,
    path: PathBuf,
    shared: Rc<RefCell<ConfigHotReloadShared>>,
) -> Result<(), NekolandError> {
    // Watch the parent directory rather than the file itself so editor save-via-rename flows
    // still trigger a reload.
    let watch_dir = watched_directory(&path);
    let watched_name = watched_file_name(&path)?;
    let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
        .map_err(nix_error_to_runtime)?;
    inotify.add_watch(&watch_dir, config_watch_mask()).map_err(nix_error_to_runtime)?;

    {
        let mut shared = shared.borrow_mut();
        shared.watcher_active = true;
    }

    handle
        .insert_source(Generic::new(inotify, Interest::READ, Mode::Level), move |_, inotify, _| {
            drain_inotify_events(inotify.as_ref(), &path, &watched_name, &shared)
        })
        .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

    Ok(())
}

fn drain_inotify_events(
    inotify: &Inotify,
    path: &Path,
    watched_name: &OsString,
    shared: &Rc<RefCell<ConfigHotReloadShared>>,
) -> Result<PostAction, std::io::Error> {
    loop {
        match inotify.read_events() {
            Ok(events) => {
                let mut relevant_change = false;
                for event in events {
                    if event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                        relevant_change = true;
                        break;
                    }

                    let matches_target = event.name.as_ref() == Some(watched_name);
                    let affects_watched_file = matches_target
                        && event.mask.intersects(
                            AddWatchFlags::IN_MODIFY
                                | AddWatchFlags::IN_CLOSE_WRITE
                                | AddWatchFlags::IN_MOVED_TO
                                | AddWatchFlags::IN_CREATE
                                | AddWatchFlags::IN_DELETE
                                | AddWatchFlags::IN_ATTRIB,
                        );
                    let affects_watched_directory = event.mask.intersects(
                        AddWatchFlags::IN_DELETE_SELF
                            | AddWatchFlags::IN_MOVE_SELF
                            | AddWatchFlags::IN_IGNORED,
                    );

                    if affects_watched_file || affects_watched_directory {
                        relevant_change = true;
                        break;
                    }
                }

                if relevant_change {
                    let mut shared = shared.borrow_mut();
                    shared.last_observed_modified = observed_modified_at(path);
                    shared.pending_change = true;
                }
            }
            Err(Errno::EAGAIN) => break,
            Err(error) => return Err(std::io::Error::from_raw_os_error(error as i32)),
        }
    }

    Ok(PostAction::Continue)
}

fn watched_directory(path: &Path) -> PathBuf {
    path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
}

fn watched_file_name(path: &Path) -> Result<OsString, NekolandError> {
    path.file_name().map(OsString::from).ok_or_else(|| {
        NekolandError::Runtime(format!("config path {} has no file name", path.display()))
    })
}

fn config_watch_mask() -> AddWatchFlags {
    AddWatchFlags::IN_MODIFY
        | AddWatchFlags::IN_CLOSE_WRITE
        | AddWatchFlags::IN_MOVED_TO
        | AddWatchFlags::IN_CREATE
        | AddWatchFlags::IN_DELETE
        | AddWatchFlags::IN_ATTRIB
        | AddWatchFlags::IN_DELETE_SELF
        | AddWatchFlags::IN_MOVE_SELF
}

fn nix_error_to_runtime(error: Errno) -> NekolandError {
    NekolandError::Runtime(error.desc().to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use bevy_ecs::error::{BevyError, DefaultErrorHandler, ErrorContext};
    use calloop::EventLoop;
    use nekoland_core::calloop::CalloopSourceRegistry;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::ExtractSchedule;
    use nekoland_ecs::resources::{CompositorConfig, ConfiguredAction, KeyboardLayoutState};

    use crate::{ConfigPlugin, LoadedConfigSource};

    use super::observed_modified_at;

    static HOT_RELOAD_ERROR_COUNT: AtomicUsize = AtomicUsize::new(0);

    const INITIAL_CONFIG: &str = r##"
[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#111111"
background_color = "#f5f5f5"

[input]
focus_follows_mouse = true
repeat_rate = 30

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Alt" = { viewport_pan_mode = true }
"Super+Return" = { exec = ["foot"] }
"Super+Q" = { close = true }
"##;

    const RELOADED_CONFIG: &str = r##"
[theme]
name = "frappe"
cursor_theme = "capitaine"
border_color = "#222222"
background_color = "#101010"

[input]
focus_follows_mouse = false
repeat_rate = 45

[[outputs]]
name = "HDMI-A-1"
mode = "2560x1440@75"
scale = 2
enabled = true

[keybinds.bindings]
"Ctrl+Shift" = { viewport_pan_mode = true }
"Super+P" = { exec = ["wlogout", "--protocol", "layer-shell"] }
"##;

    const KEYBOARD_LAYOUT_CONFIG: &str = r##"
[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#111111"
background_color = "#f5f5f5"

[input]
focus_follows_mouse = true
repeat_rate = 30

[input.keyboard]
current = "us"

[[input.keyboard.layouts]]
name = "us"
layout = "us"

[[input.keyboard.layouts]]
name = "de"
layout = "de"

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Return" = { exec = ["foot"] }
"##;

    const INVALID_CONFIG: &str = r##"
[theme]
name = "broken"
cursor_theme = "default"
"##;

    #[derive(Debug)]
    struct TempConfigFile {
        path: PathBuf,
    }

    impl Drop for TempConfigFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn hot_reload_updates_config_and_preserves_previous_value_on_invalid_reload() {
        let temp_config = TempConfigFile { path: unique_temp_path("hot-reload") };
        write_config(&temp_config.path, INITIAL_CONFIG);

        let mut app = NekolandApp::new("config-hot-reload-test");
        app.add_plugin(ConfigPlugin::new(&temp_config.path));

        {
            let world = app.inner().world();
            let Some(config) = world.get_resource::<CompositorConfig>() else {
                panic!("config should be initialized");
            };
            let Some(source) = world.get_resource::<LoadedConfigSource>() else {
                panic!("config source should be initialized");
            };

            assert_eq!(config.theme, "latte");
            assert_eq!(config.cursor_theme, "breeze");
            assert_eq!(
                config.viewport_pan_modifiers,
                nekoland_ecs::resources::ModifierMask::new(false, true, false, true)
            );
            assert_eq!(
                config.keybindings.get("Super+Return"),
                Some(&vec![ConfiguredAction::Exec { argv: vec!["foot".to_owned()] }])
            );
            assert!(source.loaded_from_disk);
            assert_eq!(source.successful_reloads, 1);
            assert!(source.last_reload_error.is_none());
        }

        rewrite_config(&temp_config.path, RELOADED_CONFIG);
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        {
            let world = app.inner().world();
            let Some(config) = world.get_resource::<CompositorConfig>() else {
                panic!("config should stay available");
            };
            let Some(source) = world.get_resource::<LoadedConfigSource>() else {
                panic!("config source should stay available");
            };

            assert_eq!(config.theme, "frappe");
            assert_eq!(config.cursor_theme, "capitaine");
            assert_eq!(
                config.viewport_pan_modifiers,
                nekoland_ecs::resources::ModifierMask::new(true, false, true, false)
            );
            assert_eq!(config.keybindings.len(), 1);
            assert_eq!(
                config.keybindings.get("Super+P"),
                Some(&vec![ConfiguredAction::Exec {
                    argv: vec![
                        "wlogout".to_owned(),
                        "--protocol".to_owned(),
                        "layer-shell".to_owned(),
                    ],
                }])
            );
            assert_eq!(source.successful_reloads, 2);
            assert!(source.last_reload_error.is_none());
        }

        rewrite_config(&temp_config.path, INVALID_CONFIG);
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let world = app.inner().world();
        let Some(config) = world.get_resource::<CompositorConfig>() else {
            panic!("config should stay available");
        };
        let Some(source) = world.get_resource::<LoadedConfigSource>() else {
            panic!("config source should stay available");
        };

        assert_eq!(config.theme, "frappe");
        assert_eq!(config.cursor_theme, "capitaine");
        assert_eq!(
            source.successful_reloads, 2,
            "failed reloads must not overwrite the successful reload count"
        );
        assert!(
            source
                .last_reload_error
                .as_deref()
                .is_some_and(|message| message.contains("parse error")),
            "invalid config should record the last reload failure"
        );
    }

    #[test]
    fn hot_reload_recovers_after_startup_fallback_when_config_file_appears() {
        let temp_config = TempConfigFile { path: unique_temp_path("startup-fallback") };

        let mut app = NekolandApp::new("config-startup-fallback-test");
        app.add_plugin(ConfigPlugin::new(&temp_config.path));

        {
            let world = app.inner().world();
            let Some(config) = world.get_resource::<CompositorConfig>() else {
                panic!("config should be initialized");
            };
            let Some(source) = world.get_resource::<LoadedConfigSource>() else {
                panic!("config source should be initialized");
            };

            assert_eq!(config.theme, "catppuccin-latte");
            assert!(!source.loaded_from_disk);
            assert_eq!(source.successful_reloads, 0);
            assert!(source.last_reload_error.is_some());
        }

        rewrite_config(&temp_config.path, INITIAL_CONFIG);
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let world = app.inner().world();
        let Some(config) = world.get_resource::<CompositorConfig>() else {
            panic!("config should reload once the file appears");
        };
        let Some(source) = world.get_resource::<LoadedConfigSource>() else {
            panic!("config source should stay available");
        };

        assert_eq!(config.theme, "latte");
        assert_eq!(config.cursor_theme, "breeze");
        assert!(source.loaded_from_disk);
        assert_eq!(source.successful_reloads, 1);
        assert!(source.last_reload_error.is_none());
    }

    #[test]
    fn invalid_reload_reports_through_fallible_system_handler() {
        let temp_config = TempConfigFile { path: unique_temp_path("fallible-hot-reload") };
        write_config(&temp_config.path, INITIAL_CONFIG);

        HOT_RELOAD_ERROR_COUNT.store(0, Ordering::Relaxed);

        let mut app = NekolandApp::new("config-fallible-hot-reload-test");
        app.add_plugin(ConfigPlugin::new(&temp_config.path));
        app.inner_mut().world_mut().insert_resource(DefaultErrorHandler(count_hot_reload_error));

        rewrite_config(&temp_config.path, INVALID_CONFIG);
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let Some(source) = app.inner().world().get_resource::<LoadedConfigSource>() else {
            panic!("config source should stay available");
        };

        assert_eq!(
            HOT_RELOAD_ERROR_COUNT.load(Ordering::Relaxed),
            1,
            "invalid config reload should be surfaced through the default error handler"
        );
        assert!(
            source
                .last_reload_error
                .as_deref()
                .is_some_and(|message| message.contains("parse error")),
            "invalid config should still record the last reload failure in ECS state"
        );
    }

    #[test]
    fn hot_reload_preserves_runtime_keyboard_layout_selection_when_still_available() {
        let temp_config = TempConfigFile { path: unique_temp_path("keyboard-layout-hot-reload") };
        write_config(&temp_config.path, KEYBOARD_LAYOUT_CONFIG);

        let mut app = NekolandApp::new("config-keyboard-layout-hot-reload-test");
        app.add_plugin(ConfigPlugin::new(&temp_config.path));

        {
            let Some(mut keyboard_layout_state) =
                app.inner_mut().world_mut().get_resource_mut::<KeyboardLayoutState>()
            else {
                panic!("keyboard layout state should be initialized");
            };
            assert!(keyboard_layout_state.activate_name("de"));
        }

        rewrite_config(&temp_config.path, KEYBOARD_LAYOUT_CONFIG);
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let Some(keyboard_layout_state) =
            app.inner().world().get_resource::<KeyboardLayoutState>()
        else {
            panic!("keyboard layout state should remain available");
        };

        assert_eq!(keyboard_layout_state.active_name(), "de");
        assert_eq!(keyboard_layout_state.layouts.len(), 2);
    }

    #[test]
    fn linux_inotify_source_reports_changes_through_calloop() {
        let temp_config = TempConfigFile { path: unique_temp_path("inotify") };
        write_config(&temp_config.path, INITIAL_CONFIG);

        let mut app = NekolandApp::new("config-inotify-test");
        app.add_plugin(ConfigPlugin::new(&temp_config.path));

        let Ok(mut event_loop) = EventLoop::try_new() else {
            panic!("calloop event loop should initialize for config tests");
        };
        {
            let Some(mut registry) =
                app.inner_mut().world_mut().get_non_send_resource_mut::<CalloopSourceRegistry>()
            else {
                panic!("config plugin should install a calloop source registry");
            };
            if let Err(error) = registry.install_all(&event_loop.handle()) {
                panic!("config watcher sources should register: {error}");
            }
        }

        rewrite_config(&temp_config.path, RELOADED_CONFIG);
        if let Err(error) = event_loop.dispatch(Duration::from_millis(50), &mut ()) {
            panic!("calloop should dispatch inotify events: {error}");
        }
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let world = app.inner().world();
        let Some(config) = world.get_resource::<CompositorConfig>() else {
            panic!("config should be available after inotify dispatch");
        };
        let Some(source) = world.get_resource::<LoadedConfigSource>() else {
            panic!("config source should stay available");
        };

        assert_eq!(config.theme, "frappe");
        assert_eq!(config.cursor_theme, "capitaine");
        assert_eq!(source.successful_reloads, 2);
        assert!(source.last_reload_error.is_none());
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let Ok(duration_since_epoch) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            panic!("system time should be after UNIX epoch");
        };
        let unique = duration_since_epoch.as_nanos();
        std::env::temp_dir().join(format!("nekoland-{prefix}-{unique}.toml"))
    }

    fn write_config(path: &Path, contents: &str) {
        if let Err(error) = fs::write(path, contents) {
            panic!("temporary config should be writable: {error}");
        }
    }

    fn rewrite_config(path: &Path, contents: &str) {
        let original = observed_modified_at(path);
        write_config(path, contents);
        for _ in 0..50 {
            if observed_modified_at(path) != original {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn count_hot_reload_error(_: BevyError, _: ErrorContext) {
        HOT_RELOAD_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}
