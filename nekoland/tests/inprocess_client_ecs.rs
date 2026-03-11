use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::components::{WindowState, WlSurfaceHandle, XdgWindow};
use nekoland_ecs::resources::{KeyboardFocusState, RenderList};
use nekoland_protocol::ProtocolServerState;

mod common;

#[test]
fn live_client_roundtrip_populates_window_entities_and_render_state() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-inprocess-runtime");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(48),
    });

    let socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<ProtocolServerState>()
            .expect("protocol server state should be available immediately after build");

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
        common::run_xdg_client_with_hold(&socket_path, Duration::from_millis(100))
    });
    app.run().expect("nekoland app should complete the configured frame budget");

    match client_thread.join().expect("client thread should exit cleanly") {
        Ok(summary) => {
            common::assert_globals_present(&summary.globals);
            assert!(summary.configure_serial > 0, "client should ack a configure");
        }
        Err(common::TestControl::Skip(reason)) => {
            eprintln!("skipping in-process ECS protocol test in restricted environment: {reason}");
            return;
        }
        Err(common::TestControl::Fail(reason)) => {
            panic!("in-process wayland client failed: {reason}");
        }
    }

    let (window_rows, render_elements, focused_surface) = {
        let world = app.inner_mut().world_mut();
        let mut windows = world.query::<(&WlSurfaceHandle, &XdgWindow, &WindowState)>();
        let window_rows = windows
            .iter(world)
            .map(|(surface, window, state)| (surface.id, window.title.clone(), state.clone()))
            .collect::<Vec<_>>();
        let render_elements = world
            .get_resource::<RenderList>()
            .expect("render list should be initialized by RenderPlugin")
            .elements
            .clone();
        let focused_surface = world
            .get_resource::<KeyboardFocusState>()
            .expect("keyboard focus state should be initialized by InputPlugin")
            .focused_surface;
        (window_rows, render_elements, focused_surface)
    };

    assert!(
        !window_rows.is_empty(),
        "live protocol traffic should create at least one XdgWindow entity"
    );

    let (surface_id, title, state) = window_rows[0].clone();
    assert_eq!(title, format!("Window {surface_id}"));
    assert_ne!(state, WindowState::Hidden, "newly mapped client window should remain visible");
    assert!(
        render_elements.iter().any(|element| element.surface_id == surface_id),
        "render list should include the client window surface: {render_elements:?}"
    );
    assert!(
        render_elements.iter().any(|element| element.surface_id == 0),
        "render list should still include the cursor element: {render_elements:?}"
    );
    assert_eq!(
        focused_surface,
        Some(surface_id),
        "focus manager should focus the first visible client window"
    );
}

fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}
