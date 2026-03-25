//! In-process integration test for focus-change subscription events.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    SurfaceGeometry, WindowLayout, WindowMode, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::KeyboardFocusState;
use nekoland_ipc::commands::{QueryCommand, TreeSnapshot, WindowCommand};
use nekoland_ipc::{
    FocusChangeSnapshot, IpcCommand, IpcRequest, IpcServerState, IpcSubscription,
    SubscriptionTopic, send_request_to_path, subscribe_to_path,
};

mod common;

/// Surface id of the window that starts focused.
const PRIMARY_SURFACE_ID: u64 = 101;
/// Surface id of the window the test later focuses through IPC.
const TARGET_SURFACE_ID: u64 = 202;

/// Verifies that a focus request results in a `focus_changed` subscription event with both the
/// previous and new focused surfaces.
#[test]
fn focus_subscription_reports_window_focus_transitions() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _runtime_dir = common::RuntimeDirGuard::new("nekoland-focus-subscription");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(160),
    });
    app.inner_mut()
        .insert_resource(KeyboardFocusState { focused_surface: Some(PRIMARY_SURFACE_ID) });
    seed_windows(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping focus subscription test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let subscription_path = ipc_socket_path.clone();
    let subscription_thread = thread::spawn(move || {
        wait_for_focus_change(
            &subscription_path,
            IpcSubscription {
                topic: SubscriptionTopic::Focus,
                include_payloads: true,
                events: vec!["focus_changed".to_owned()],
            },
            TARGET_SURFACE_ID,
        )
    });

    let command_thread =
        thread::spawn(move || issue_focus_command_when_windows_are_ready(&ipc_socket_path));
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    match command_thread.join() {
        Ok(result) => match result {
            Ok(()) => {}
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping focus subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("focus command sequence failed: {reason}");
            }
        },
        Err(_) => panic!("focus command thread should exit cleanly"),
    };

    let focus_change = match subscription_thread.join() {
        Ok(result) => match result {
            Ok(focus_change) => focus_change,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping focus subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("focus subscription failed: {reason}");
            }
        },
        Err(_) => panic!("subscription thread should exit cleanly"),
    };

    assert_eq!(focus_change.previous_surface, Some(PRIMARY_SURFACE_ID));
    assert_eq!(focus_change.focused_surface, Some(TARGET_SURFACE_ID));
}

/// Seeds two windows so the focus request has deterministic targets.
fn seed_windows(world: &mut bevy_ecs::world::World) {
    for (surface_id, title, x) in
        [(PRIMARY_SURFACE_ID, "Focus Window 1", 0), (TARGET_SURFACE_ID, "Focus Window 2", 480)]
    {
        world.spawn((WindowBundle {
            surface: WlSurfaceHandle { id: surface_id },
            geometry: SurfaceGeometry { x, y: 32, width: 440, height: 700 },
            window: XdgWindow { app_id: "org.nekoland.focus".to_owned(), title: title.to_owned() },
            layout: WindowLayout::Tiled,
            mode: WindowMode::Normal,
            ..Default::default()
        },));
    }
}

/// Returns the default config path used by this integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

/// Waits until both seeded windows appear in the tree query, then issues the focus request.
fn issue_focus_command_when_windows_are_ready(
    socket_path: &Path,
) -> Result<(), common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        match query_tree(socket_path) {
            Ok(tree)
                if tree.windows.iter().any(|window| window.surface_id == PRIMARY_SURFACE_ID)
                    && tree.windows.iter().any(|window| window.surface_id == TARGET_SURFACE_ID) =>
            {
                let reply = send_request_to_path(
                    socket_path,
                    &IpcRequest {
                        correlation_id: 7,
                        command: IpcCommand::Window(WindowCommand::Focus {
                            surface_id: TARGET_SURFACE_ID,
                        }),
                    },
                )
                .map_err(classify_ipc_error)?;

                if !reply.ok {
                    return Err(common::TestControl::Fail(format!(
                        "IPC focus request was rejected: {reply:?}"
                    )));
                }
                return Ok(());
            }
            Ok(_) => {}
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) => return Err(classify_ipc_error(error)),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for seeded windows before issuing focus request".to_owned(),
            ));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits for the `focus_changed` event targeting the expected surface.
fn wait_for_focus_change(
    socket_path: &Path,
    subscription: IpcSubscription,
    expected_surface: u64,
) -> Result<FocusChangeSnapshot, common::TestControl> {
    let mut stream = subscribe_to_path(socket_path, &subscription).map_err(classify_ipc_error)?;
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        match stream.read_event() {
            Ok(event) => {
                // The focus topic may emit other transitions first; keep
                // reading until the expected focused surface shows up.
                let Some(payload) = event.payload else {
                    continue;
                };
                let focus_change =
                    serde_json::from_value::<FocusChangeSnapshot>(payload).map_err(|error| {
                        common::TestControl::Fail(format!(
                            "failed to decode focus_changed payload: {error}"
                        ))
                    })?;
                if focus_change.focused_surface == Some(expected_surface) {
                    return Ok(focus_change);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) => return Err(classify_ipc_error(error)),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for focus_changed subscription event".to_owned(),
            ));
        }
    }
}

/// Queries the current tree snapshot over IPC.
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

/// Maps IPC failures into the test's skip/fail control flow.
fn classify_ipc_error(error: std::io::Error) -> common::TestControl {
    if ipc_error_is_skippable(&error) {
        return common::TestControl::Skip(error.to_string());
    }

    common::TestControl::Fail(error.to_string())
}

/// Identifies retryable transient IPC errors.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
    ) || error.raw_os_error() == Some(11)
}

/// Identifies IPC errors that should skip the test in restricted environments.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}
