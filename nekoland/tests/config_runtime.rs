use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nekoland_backend::BackendPlugin;
use nekoland_config::ConfigPlugin;
use nekoland_core::prelude::NekolandApp;
use nekoland_core::schedules::{ExtractSchedule, InputSchedule, LayoutSchedule};
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, LayoutSlot, OutputDevice, OutputProperties, ServerDecoration, SurfaceGeometry,
    WindowState, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    CommandHistoryState, CompositorClock, GlobalPointerPosition, KeyboardFocusState,
    PendingXdgRequests, WindowLifecycleAction, WindowLifecycleRequest, XdgSurfaceRole,
};
use nekoland_input::InputPlugin;
use nekoland_shell::ShellPlugin;

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
"Super+Return" = "spawn-terminal"
"##;

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
"Super+P" = "show-power-menu"
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
        world.spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 101 },
                geometry: SurfaceGeometry { x: 0, y: 32, width: 320, height: 240 },
                window: XdgWindow {
                    app_id: "org.nekoland.config".to_owned(),
                    title: "Primary".to_owned(),
                    last_acked_configure: None,
                },
                state: WindowState::Floating,
                decoration: ServerDecoration { enabled: true },
                border_theme: BorderTheme::default(),
                ..Default::default()
            },
            LayoutSlot { workspace: 1, column: 0, row: 0 },
        ));
        world.spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 202 },
                geometry: SurfaceGeometry { x: 400, y: 48, width: 320, height: 240 },
                window: XdgWindow {
                    app_id: "org.nekoland.config".to_owned(),
                    title: "Secondary".to_owned(),
                    last_acked_configure: None,
                },
                state: WindowState::Floating,
                decoration: ServerDecoration { enabled: true },
                border_theme: BorderTheme::default(),
                ..Default::default()
            },
            LayoutSlot { workspace: 1, column: 0, row: 0 },
        ));
    }

    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(InputSchedule);
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    {
        let world = app.inner_mut().world_mut();
        let focused_surface = world
            .get_resource::<KeyboardFocusState>()
            .expect("focus state should exist")
            .focused_surface;
        let outputs = world
            .query::<(&OutputDevice, &OutputProperties)>()
            .iter(world)
            .map(|(output, properties)| (output.name.clone(), properties.clone()))
            .collect::<Vec<_>>();
        let history_limit = world
            .get_resource::<CommandHistoryState>()
            .expect("command history state should exist")
            .limit;
        let border_colors = world
            .query::<&BorderTheme>()
            .iter(world)
            .map(|border| border.color.clone())
            .collect::<Vec<_>>();

        assert_eq!(focused_surface, Some(101));
        assert_eq!(outputs.len(), 1, "initial config should produce exactly one configured output");
        assert_eq!(outputs[0].0, "eDP-1");
        assert_eq!(outputs[0].1.width, 1920);
        assert_eq!(outputs[0].1.height, 1080);
        assert_eq!(outputs[0].1.refresh_millihz, 60_000);
        assert_eq!(outputs[0].1.scale, 1);
        assert_eq!(history_limit, 7);
        assert!(
            border_colors.iter().all(|color| color == "#112233"),
            "initial config border color should be applied to all existing windows: {border_colors:?}"
        );
    }

    rewrite_config(&temp_config.path, RELOADED_CONFIG);
    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(ExtractSchedule);
    app.inner_mut().world_mut().run_schedule(InputSchedule);
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    {
        let world = app.inner_mut().world_mut();
        let focused_surface = world
            .get_resource::<KeyboardFocusState>()
            .expect("focus state should exist")
            .focused_surface;
        let outputs = world
            .query::<(&OutputDevice, &OutputProperties)>()
            .iter(world)
            .map(|(output, properties)| (output.name.clone(), properties.clone()))
            .collect::<Vec<_>>();
        let history_limit = world
            .get_resource::<CommandHistoryState>()
            .expect("command history state should exist")
            .limit;
        let border_colors = world
            .query::<&BorderTheme>()
            .iter(world)
            .map(|border| border.color.clone())
            .collect::<Vec<_>>();

        assert_eq!(focused_surface, Some(202));
        assert_eq!(outputs.len(), 1, "reloaded config should converge to one configured output");
        assert_eq!(outputs[0].0, "HDMI-A-1");
        assert_eq!(outputs[0].1.width, 2560);
        assert_eq!(outputs[0].1.height, 1440);
        assert_eq!(outputs[0].1.refresh_millihz, 75_000);
        assert_eq!(outputs[0].1.scale, 2);
        assert_eq!(history_limit, 3);
        assert!(
            border_colors.iter().all(|color| color == "#445566"),
            "hot-reloaded border color should be applied to all existing windows: {border_colors:?}"
        );
    }

    app.inner_mut()
        .world_mut()
        .get_resource_mut::<PendingXdgRequests>()
        .expect("shell plugin should initialize the XDG request queue")
        .items
        .push(WindowLifecycleRequest {
            surface_id: 303,
            action: WindowLifecycleAction::Committed { role: XdgSurfaceRole::Toplevel, size: None },
        });
    app.inner_mut().world_mut().run_schedule(LayoutSchedule);

    let world = app.inner_mut().world_mut();
    let created_window = world
        .query::<(&WlSurfaceHandle, &WindowState, &BorderTheme)>()
        .iter(world)
        .find(|(surface, _, _)| surface.id == 303)
        .map(|(_, state, border)| (state.clone(), border.clone()))
        .expect("committed toplevel should spawn a new shell window");

    assert_eq!(created_window.0, WindowState::Floating);
    assert_eq!(created_window.1.color, "#445566");
}

fn unique_temp_path(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("nekoland-{prefix}-{unique}.toml"))
}

fn write_config(path: &Path, contents: &str) {
    fs::write(path, contents).expect("temporary config should be writable");
}

fn rewrite_config(path: &Path, contents: &str) {
    let original = fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok());
    write_config(path, contents);
    for _ in 0..50 {
        let modified = fs::metadata(path).ok().and_then(|metadata| metadata.modified().ok());
        if modified != original {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
