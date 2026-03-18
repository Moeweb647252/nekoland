//! In-process integration test that verifies live Wayland client traffic materializes ECS window
//! state and render-plan output.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::components::{
    WindowDisplayState, WindowLayout, WindowMode, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    CursorSceneSnapshot, KeyboardFocusState, RenderPlan, RenderPlanItem,
};
use nekoland_protocol::ProtocolServerState;

mod common;

/// Verifies that a live client round-trip creates window entities, render-plan items, and focus.
#[test]
fn live_client_roundtrip_populates_window_entities_and_render_state() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-inprocess-runtime");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(48),
    });

    let socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<ProtocolServerState>() else {
            panic!("protocol server state should be available immediately after build");
        };

        match (&server_state.socket_name, &server_state.startup_error) {
            (Some(socket_name), _) => runtime_dir.path.join(socket_name),
            (None, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!(
                    "skipping in-process ECS protocol test in restricted environment: {error}"
                );
                return;
            }
            (None, Some(error)) => panic!("protocol startup failed before run: {error}"),
            (None, None) => panic!("protocol startup produced neither socket nor error"),
        }
    };

    let client_thread = thread::spawn(move || {
        // Keep the client alive briefly after the first configure so the
        // compositor has enough frames to materialize ECS and render state.
        common::run_xdg_client_with_hold(&socket_path, Duration::from_millis(100))
    });
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    match client_thread.join() {
        Ok(result) => match result {
            Ok(summary) => {
                common::assert_globals_present(&summary.globals);
                assert!(summary.configure_serial > 0, "client should ack a configure");
            }
            Err(common::TestControl::Skip(reason)) => {
                eprintln!(
                    "skipping in-process ECS protocol test in restricted environment: {reason}"
                );
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("in-process wayland client failed: {reason}");
            }
        },
        Err(_) => panic!("client thread should exit cleanly"),
    }

    let (window_rows, render_surface_ids, cursor_state, focused_surface) = {
        let world = app.inner_mut().world_mut();
        let mut windows =
            world.query::<(&WlSurfaceHandle, &XdgWindow, &WindowLayout, &WindowMode)>();
        let window_rows = windows
            .iter(world)
            .map(|(surface, window, layout, mode)| {
                (
                    surface.id,
                    window.title.clone(),
                    WindowDisplayState::from_layout_mode(*layout, *mode),
                )
            })
            .collect::<Vec<_>>();
        let Some(render_plan) = world.get_resource::<RenderPlan>() else {
            panic!("render plan should be initialized by RenderPlugin");
        };
        let render_surface_ids = render_plan
            .outputs
            .values()
            .flat_map(|output_plan| output_plan.iter_ordered())
            .filter_map(|item| match item {
                RenderPlanItem::Surface(item) => Some(item.surface_id),
                RenderPlanItem::SolidRect(_)
                | RenderPlanItem::Backdrop(_)
                | RenderPlanItem::Cursor(_) => None,
            })
            .collect::<Vec<_>>();
        let Some(cursor_state) = world.get_resource::<CursorSceneSnapshot>() else {
            panic!("cursor scene snapshot should be initialized by RenderPlugin");
        };
        let cursor_state = cursor_state.clone();
        let Some(keyboard_focus) = world.get_resource::<KeyboardFocusState>() else {
            panic!("keyboard focus state should be initialized by InputPlugin");
        };
        let focused_surface = keyboard_focus.focused_surface;
        (window_rows, render_surface_ids, cursor_state, focused_surface)
    };

    assert!(
        !window_rows.is_empty(),
        "live protocol traffic should create at least one XdgWindow entity"
    );

    let (surface_id, title, state) = window_rows[0].clone();
    assert_eq!(title, format!("Window {surface_id}"));
    assert_ne!(
        state,
        WindowDisplayState::Hidden,
        "newly mapped client window should remain visible"
    );
    assert!(
        render_surface_ids.contains(&surface_id),
        "render plan should include the client window surface: {render_surface_ids:?}"
    );
    assert!(cursor_state.visible, "cursor scene snapshot should stay visible: {cursor_state:?}");
    assert_eq!(
        focused_surface,
        Some(surface_id),
        "focus manager should focus the first visible client window"
    );
}

/// Returns the default config path used by this integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}
