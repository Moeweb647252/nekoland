//! In-process integration test for config hot reload and its effect on runtime shell/backend
//! defaults.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nekoland_backend::BackendPlugin;
use nekoland_config::{ConfigPlugin, resources::CompositorConfig};
use nekoland_core::prelude::NekolandApp;
use nekoland_core::schedules::{ExtractSchedule, InputSchedule, LayoutSchedule};
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, ServerDecoration, SurfaceGeometry, WindowDisplayState, WindowLayout,
    WindowManagementHints, WindowMode, WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    CommandHistoryState, CompositorClock, GlobalPointerPosition, KeyboardFocusState, ModifierMask,
    WaylandIngress, WindowEvent, WindowEventRequest,
};
use nekoland_input::InputPlugin;
use nekoland_shell::ShellPlugin;

/// Baseline config loaded before the runtime rewrite.
const INITIAL_CONFIG: &str = r##"
default_layout = "tiling"

[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#112233"
background_color = "#f5f5f5"

[input]
focus_follows_mouse = false
repeat_rate = 30

[ipc]
command_history_limit = 7

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Alt" = { viewport_pan_mode = true }
"Super+Return" = { exec = ["foot"] }
"##;

/// Replacement config written while the app is already running.
const RELOADED_CONFIG: &str = r##"
default_layout = "floating"

[theme]
name = "frappe"
cursor_theme = "capitaine"
border_color = "#445566"
background_color = "#101010"

[input]
focus_follows_mouse = true
repeat_rate = 45

[ipc]
command_history_limit = 3

[[outputs]]
name = "HDMI-A-1"
mode = "2560x1440@75"
scale = 2
enabled = true

[keybinds.bindings]
"Ctrl+Shift" = { viewport_pan_mode = true }
"Super+P" = { exec = ["wlogout", "--protocol", "layer-shell"] }
"##;

/// Temporary config file owned by the test.
#[derive(Debug)]
struct TempConfigFile {
    path: PathBuf,
}

impl Drop for TempConfigFile {
    /// Best-effort cleanup for the temporary config file used by this test.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Verifies that runtime config changes preserve existing focus while updating output config,
/// border theme, and the defaults used for newly created windows.
#[test]
fn config_runtime_updates_focus_border_and_new_window_defaults() {
    let temp_config = TempConfigFile { path: unique_temp_path("runtime-config") };
    write_config(&temp_config.path, INITIAL_CONFIG);

    let mut app = NekolandApp::new("config-runtime-test");
    app.insert_resource(CompositorClock::default())
        .insert_resource(KeyboardFocusState { focused_surface: Some(101) })
        .insert_resource(GlobalPointerPosition { x: 520.0, y: 96.0 })
        .add_plugin(ConfigPlugin::new(&temp_config.path))
        .add_plugin(BackendPlugin)
        .add_plugin(InputPlugin)
        .add_plugin(ShellPlugin);

    {
        let world = app.inner_mut().world_mut();
        world.spawn((WindowBundle {
            surface: WlSurfaceHandle { id: 101 },
            geometry: SurfaceGeometry { x: 0, y: 32, width: 320, height: 240 },
            window: XdgWindow {
                app_id: "org.nekoland.config".to_owned(),
                title: "Primary".to_owned(),
            },
            layout: WindowLayout::Tiled,
            mode: WindowMode::Normal,
            decoration: ServerDecoration { enabled: true },
            border_theme: BorderTheme::default(),
            ..Default::default()
        },));
        world.spawn((WindowBundle {
            surface: WlSurfaceHandle { id: 202 },
            geometry: SurfaceGeometry { x: 400, y: 48, width: 320, height: 240 },
            window: XdgWindow {
                app_id: "org.nekoland.config".to_owned(),
                title: "Secondary".to_owned(),
            },
            layout: WindowLayout::Tiled,
            mode: WindowMode::Normal,
            decoration: ServerDecoration { enabled: true },
            border_theme: BorderTheme::default(),
            ..Default::default()
        },));
    }

    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(InputSchedule);
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    {
        let world = app.inner_mut().world_mut();
        let Some(focus_state) = world.get_resource::<KeyboardFocusState>() else {
            panic!("focus state should exist");
        };
        let focused_surface = focus_state.focused_surface;
        let Some(command_history) = world.get_resource::<CommandHistoryState>() else {
            panic!("command history state should exist");
        };
        let history_limit = command_history.limit;
        let Some(config) = world.get_resource::<CompositorConfig>() else {
            panic!("config should exist");
        };
        let configured_outputs = config.outputs.clone();
        let viewport_pan_modifiers = config.viewport_pan_modifiers;
        let border_colors = world
            .query::<&BorderTheme>()
            .iter(world)
            .map(|border| border.color.clone())
            .collect::<Vec<_>>();

        assert_eq!(focused_surface, Some(101));
        assert_eq!(configured_outputs.len(), 1, "initial config should contain one configured output");
        assert_eq!(configured_outputs[0].name, "eDP-1");
        assert_eq!(configured_outputs[0].mode.as_str(), "1920x1080@60");
        assert_eq!(configured_outputs[0].scale, 1);
        assert_eq!(history_limit, 7);
        assert_eq!(viewport_pan_modifiers, ModifierMask::new(false, true, false, true));
        assert!(
            border_colors.iter().all(|color| color == "#112233"),
            "initial config border color should be applied to all existing windows: {border_colors:?}"
        );
    }

    rewrite_config(&temp_config.path, RELOADED_CONFIG);
    // Run extract multiple times so the hot-reload watcher has a chance to see
    // the file timestamp change before input/layout consume the refreshed config.
    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(InputSchedule);
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    {
        let world = app.inner_mut().world_mut();
        let Some(focus_state) = world.get_resource::<KeyboardFocusState>() else {
            panic!("focus state should exist");
        };
        let focused_surface = focus_state.focused_surface;
        let Some(command_history) = world.get_resource::<CommandHistoryState>() else {
            panic!("command history state should exist");
        };
        let history_limit = command_history.limit;
        let Some(config) = world.get_resource::<CompositorConfig>() else {
            panic!("config should exist");
        };
        let configured_outputs = config.outputs.clone();
        let viewport_pan_modifiers = config.viewport_pan_modifiers;
        let border_colors = world
            .query::<&BorderTheme>()
            .iter(world)
            .map(|border| border.color.clone())
            .collect::<Vec<_>>();

        assert_eq!(focused_surface, Some(101));
        assert_eq!(configured_outputs.len(), 1, "reloaded config should converge to one configured output");
        assert_eq!(configured_outputs[0].name, "HDMI-A-1");
        assert_eq!(configured_outputs[0].mode.as_str(), "2560x1440@75");
        assert_eq!(configured_outputs[0].scale, 2);
        assert_eq!(history_limit, 3);
        assert_eq!(viewport_pan_modifiers, ModifierMask::new(true, false, true, false));
        assert!(
            border_colors.iter().all(|color| color == "#445566"),
            "hot-reloaded border color should be applied to all existing windows: {border_colors:?}"
        );
    }

    app.inner_mut()
        .world_mut()
        .resource_mut::<WaylandIngress>()
        .pending_window_events
        .push(WindowEventRequest {
            surface_id: 303,
            action: WindowEvent::Upsert {
                title: Some("Reloaded".to_owned()),
                app_id: Some("org.nekoland.config".to_owned()),
                hints: WindowManagementHints::native_wayland(),
                scene_geometry: Some(WindowSceneGeometry {
                    x: 0,
                    y: 0,
                    width: 960,
                    height: 720,
                }),
                attached: false,
            },
        });
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    let world = app.inner_mut().world_mut();
    let created_window = world
        .query::<(&WlSurfaceHandle, &WindowLayout, &WindowMode, &BorderTheme)>()
        .iter(world)
        .find(|(surface, _, _, _)| surface.id == 303)
        .map(|(_, layout, mode, border)| {
            (WindowDisplayState::from_layout_mode(*layout, *mode), border.clone())
        })
        .unwrap_or_else(|| panic!("committed toplevel should spawn a new shell window"));

    assert_eq!(created_window.0, WindowDisplayState::Floating);
    assert_eq!(created_window.1.color, "#445566");
}

#[test]
fn tiling_default_layout_splits_new_windows_across_work_area() {
    let temp_config = TempConfigFile { path: unique_temp_path("runtime-tiling-config") };
    write_config(&temp_config.path, INITIAL_CONFIG);

    let mut app = NekolandApp::new("config-runtime-tiling-test");
    app.insert_resource(CompositorClock::default())
        .insert_resource(KeyboardFocusState::default())
        .insert_resource(GlobalPointerPosition { x: 0.0, y: 0.0 })
        .add_plugin(ConfigPlugin::new(&temp_config.path))
        .add_plugin(BackendPlugin)
        .add_plugin(InputPlugin)
        .add_plugin(ShellPlugin);

    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(InputSchedule);
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    app.inner_mut().world_mut().resource_mut::<WaylandIngress>().pending_window_events.push(
        WindowEventRequest {
        surface_id: 401,
        action: WindowEvent::Upsert {
            title: Some("Window 401".to_owned()),
            app_id: Some("org.nekoland.config".to_owned()),
            hints: WindowManagementHints::native_wayland(),
            scene_geometry: Some(WindowSceneGeometry {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            }),
            attached: true,
        },
    });
    app.inner_mut().world_mut().resource_mut::<WaylandIngress>().pending_window_events.push(
        WindowEventRequest {
        surface_id: 402,
        action: WindowEvent::Upsert {
            title: Some("Window 402".to_owned()),
            app_id: Some("org.nekoland.config".to_owned()),
            hints: WindowManagementHints::native_wayland(),
            scene_geometry: Some(WindowSceneGeometry {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            }),
            attached: true,
        },
    });
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    let world = app.inner_mut().world_mut();
    let Some(work_area) = world.get_resource::<nekoland_ecs::resources::WorkArea>() else {
        panic!("work area should exist");
    };
    let work_area = *work_area;
    let mut windows =
        world.query::<(&WlSurfaceHandle, &SurfaceGeometry, &WindowLayout, &WindowMode)>();
    let window_rows = windows
        .iter(world)
        .filter(|(surface, _, _, _)| surface.id == 401 || surface.id == 402)
        .map(|(surface, geometry, layout, mode)| {
            (surface.id, geometry.clone(), WindowDisplayState::from_layout_mode(*layout, *mode))
        })
        .collect::<Vec<_>>();

    assert_eq!(window_rows.len(), 2, "two committed toplevels should exist");

    let first = window_rows
        .iter()
        .find(|(surface_id, _, _)| *surface_id == 401)
        .unwrap_or_else(|| panic!("first tiled window should exist"));
    let second = window_rows
        .iter()
        .find(|(surface_id, _, _)| *surface_id == 402)
        .unwrap_or_else(|| panic!("second tiled window should exist"));

    assert_eq!(first.2, WindowDisplayState::Tiled);
    assert_eq!(second.2, WindowDisplayState::Tiled);
    assert_eq!(
        (first.1.x, first.1.y, first.1.width, first.1.height),
        (work_area.x, work_area.y, work_area.width / 2, work_area.height)
    );
    assert_eq!(
        (second.1.x, second.1.y, second.1.width, second.1.height),
        (
            work_area.x + (work_area.width / 2) as i32,
            work_area.y,
            work_area.width - (work_area.width / 2),
            work_area.height,
        )
    );
}

/// Creates a unique temporary config path.
fn unique_temp_path(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| panic!("system time should be after UNIX epoch"))
        .as_nanos();
    std::env::temp_dir().join(format!("nekoland-{prefix}-{unique}.toml"))
}

/// Writes the supplied config contents to disk.
fn write_config(path: &Path, contents: &str) {
    if let Err(error) = fs::write(path, contents) {
        panic!("temporary config should be writable: {error}");
    }
}

/// Rewrites the config file and waits until the filesystem modification timestamp changes.
fn rewrite_config(path: &Path, contents: &str) {
    let original = fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok());
    write_config(path, contents);
    for _ in 0..50 {
        // The watcher keys off file modification time, so avoid returning until
        // the filesystem reports a different timestamp.
        let modified = fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok());
        if modified != original {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
