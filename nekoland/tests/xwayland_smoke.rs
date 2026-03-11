use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::{Res, Resource};
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::ExtractSchedule;
use nekoland_ipc::commands::{QueryCommand, TreeSnapshot, WindowSnapshot};
use nekoland_ipc::{IpcCommand, IpcRequest, IpcServerState, send_request_to_path};
use nekoland_protocol::XWaylandServerState;
use smithay::reexports::x11rb::connection::Connection;
use smithay::reexports::x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use smithay::reexports::x11rb::wrapper::ConnectionExt as _;

mod common;

const WINDOW_TITLE: &str = "Nekoland X11 Smoke";
const WINDOW_CLASS: &[u8] = b"nekoland-x11-smoke\0nekoland-x11-smoke\0";

#[derive(Debug, Clone, Resource)]
struct XWaylandDisplayProbe(Arc<Mutex<Option<Result<String, String>>>>);

#[test]
fn xwayland_window_appears_in_tree_snapshot() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let _runtime_dir = common::RuntimeDirGuard::new("nekoland-xwayland-smoke");
    let config_path = workspace_config_path();
    let display_probe = XWaylandDisplayProbe(Arc::new(Mutex::new(None)));

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(2),
        max_frames: Some(320),
    });
    app.insert_resource(display_probe.clone());
    app.inner_mut().add_systems(ExtractSchedule, record_xwayland_display_system);

    let ipc_socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<IpcServerState>()
            .expect("IPC server state should be available immediately after build");

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping XWayland smoke test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let probe = display_probe.0.clone();
    let client_thread = thread::spawn(move || run_x11_smoke_client(probe, &ipc_socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let x11_window =
        match client_thread.join().expect("X11 smoke client thread should exit cleanly") {
            Ok(window) => window,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping XWayland smoke test: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("XWayland smoke client failed: {reason}");
            }
        };

    let xwayland_state = app
        .inner()
        .world()
        .get_resource::<XWaylandServerState>()
        .expect("xwayland server state should be present after run");

    assert!(xwayland_state.ready, "xwayland should be ready after a successful smoke test");
    assert!(x11_window.xwayland, "tree snapshot should mark X11 clients as xwayland");
    assert_eq!(x11_window.title, WINDOW_TITLE);
    assert_eq!(x11_window.app_id, "nekoland-x11-smoke");
    assert!(
        x11_window.x11_window_id.is_some(),
        "tree snapshot should expose the originating X11 window id: {x11_window:?}"
    );
}

fn record_xwayland_display_system(
    xwayland_state: Res<XWaylandServerState>,
    display_probe: Res<XWaylandDisplayProbe>,
) {
    let mut slot =
        display_probe.0.lock().expect("xwayland display probe mutex should not be poisoned");
    if slot.is_some() {
        return;
    }

    if xwayland_state.ready {
        if let Some(display_name) = xwayland_state.display_name.clone() {
            *slot = Some(Ok(display_name));
        }
        return;
    }

    if let Some(error) = xwayland_state.startup_error.clone() {
        *slot = Some(Err(error));
    }
}

fn run_x11_smoke_client(
    display_probe: Arc<Mutex<Option<Result<String, String>>>>,
    socket_path: &Path,
) -> Result<WindowSnapshot, common::TestControl> {
    let display_name = wait_for_xwayland_display(&display_probe)?;
    let (connection, screen_num) = wait_for_x11_connection(&display_name)?;
    let screen = &connection.setup().roots[screen_num];
    let window_id = connection.generate_id().map_err(|error| {
        common::TestControl::Fail(format!("failed to allocate X11 window id: {error}"))
    })?;

    let aux = CreateWindowAux::new()
        .background_pixel(screen.white_pixel)
        .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::EXPOSURE);

    connection
        .create_window(
            screen.root_depth,
            window_id,
            screen.root,
            64,
            48,
            320,
            240,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &aux,
        )
        .map_err(|error| {
            common::TestControl::Fail(format!("failed to create X11 smoke window: {error}"))
        })?;
    connection
        .change_property8(
            PropMode::REPLACE,
            window_id,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            WINDOW_TITLE.as_bytes(),
        )
        .map_err(|error| common::TestControl::Fail(format!("failed to set WM_NAME: {error}")))?;
    connection
        .change_property8(
            PropMode::REPLACE,
            window_id,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            WINDOW_CLASS,
        )
        .map_err(|error| common::TestControl::Fail(format!("failed to set WM_CLASS: {error}")))?;
    connection.map_window(window_id).map_err(|error| {
        common::TestControl::Fail(format!("failed to map X11 smoke window: {error}"))
    })?;
    connection.flush().map_err(|error| {
        common::TestControl::Fail(format!("failed to flush X11 smoke window: {error}"))
    })?;

    let window = wait_for_x11_window(socket_path, window_id)?;

    // Keep the client mapped long enough for the compositor to finish the frame budget.
    thread::sleep(Duration::from_millis(350));
    Ok(window)
}

fn wait_for_xwayland_display(
    display_probe: &Arc<Mutex<Option<Result<String, String>>>>,
) -> Result<String, common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(3);

    loop {
        let state = display_probe
            .lock()
            .expect("xwayland display probe mutex should not be poisoned")
            .clone();

        if let Some(state) = state {
            return match state {
                Ok(display_name) => Ok(display_name),
                Err(error) if xwayland_startup_error_is_skippable(&error) => {
                    Err(common::TestControl::Skip(error))
                }
                Err(error) => Err(common::TestControl::Fail(error)),
            };
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for XWayland display name".to_owned(),
            ));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_x11_connection(
    display_name: &str,
) -> Result<(smithay::reexports::x11rb::rust_connection::RustConnection, usize), common::TestControl>
{
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        match smithay::reexports::x11rb::connect(Some(display_name)) {
            Ok(connection) => return Ok(connection),
            Err(error) => {
                let error = error.to_string();
                if x11_connect_error_is_retryable(&error) && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                if x11_connect_error_is_skippable(&error) {
                    return Err(common::TestControl::Skip(error));
                }
                return Err(common::TestControl::Fail(error));
            }
        }
    }
}

fn wait_for_x11_window(
    socket_path: &Path,
    expected_window_id: u32,
) -> Result<WindowSnapshot, common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(3);

    loop {
        match query_tree(socket_path) {
            Ok(tree) => {
                if let Some(window) = tree.windows.into_iter().find(|window| {
                    window.xwayland
                        && window.title == WINDOW_TITLE
                        && window.app_id == "nekoland-x11-smoke"
                        && window.x11_window_id == Some(expected_window_id)
                }) {
                    return Ok(window);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) => return Err(classify_ipc_error(error)),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(format!(
                "timed out waiting for mapped X11 window {expected_window_id} to appear in the IPC tree"
            )));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn query_tree(socket_path: &Path) -> Result<TreeSnapshot, std::io::Error> {
    let reply = send_request_to_path(
        socket_path,
        &IpcRequest { correlation_id: 1, command: IpcCommand::Query(QueryCommand::GetTree) },
    )?;

    if !reply.ok {
        return Err(std::io::Error::other(format!("IPC tree query failed: {}", reply.message)));
    }

    let payload = reply.payload.ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidData, "IPC tree query returned no payload")
    })?;
    serde_json::from_value(payload).map_err(std::io::Error::other)
}

fn classify_ipc_error(error: std::io::Error) -> common::TestControl {
    if ipc_error_is_skippable(&error) {
        return common::TestControl::Skip(error.to_string());
    }

    common::TestControl::Fail(error.to_string())
}

fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
    )
}

fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}

fn x11_connect_error_is_retryable(error: &str) -> bool {
    error.contains("Connection refused")
        || error.contains("No such file or directory")
        || error.contains("timed out")
}

fn x11_connect_error_is_skippable(error: &str) -> bool {
    error.contains("Permission denied") || error.contains("Operation not permitted")
}

fn xwayland_startup_error_is_skippable(error: &str) -> bool {
    error.contains("No such file or directory")
        || error.contains("not found")
        || error.contains("Permission denied")
        || error.contains("Operation not permitted")
}

fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}
