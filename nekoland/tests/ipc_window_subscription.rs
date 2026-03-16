//! In-process integration test for window geometry/state subscription events.

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
use nekoland_ipc::commands::{QueryCommand, TreeSnapshot, WindowCommand, WindowSnapshot};
use nekoland_ipc::{
    IpcCommand, IpcRequest, IpcServerState, IpcSubscription, SubscriptionTopic,
    WindowGeometryChangeSnapshot, WindowStateChangeSnapshot, send_request_to_path,
    subscribe_to_path,
};

mod common;

/// Surface id of the window whose move should trigger subscription events.
const TARGET_SURFACE_ID: u64 = 101;

/// Summary returned by the IPC command helper before the subscription events arrive.
#[derive(Debug)]
struct WindowChangeCommandSummary {
    /// Snapshot observed before the move request is sent.
    initial_window: WindowSnapshot,
    /// Requested x coordinate for the move command.
    target_x: i32,
    /// Requested y coordinate for the move command.
    target_y: i32,
}

/// Pair of window subscription events that the scenario waits for.
#[derive(Debug)]
struct WindowChangeEvents {
    geometry: WindowGeometryChangeSnapshot,
    state: WindowStateChangeSnapshot,
}

/// Verifies that a move request produces both geometry-change and state-change subscription
/// events.
#[test]
fn window_subscription_reports_geometry_and_state_transitions() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _runtime_dir = common::RuntimeDirGuard::new("nekoland-window-subscription");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(160),
    });
    seed_window(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping window subscription test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let subscription_path = ipc_socket_path.clone();
    let subscription_thread = thread::spawn(move || {
        wait_for_window_change_events(
            &subscription_path,
            IpcSubscription {
                topic: SubscriptionTopic::Window,
                include_payloads: true,
                events: vec![
                    "window_geometry_changed".to_owned(),
                    "window_state_changed".to_owned(),
                ],
            },
            TARGET_SURFACE_ID,
        )
    });

    let command_thread =
        thread::spawn(move || issue_move_command_when_window_is_ready(&ipc_socket_path));
    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let summary = match command_thread.join() {
        Ok(summary) => match summary {
            Ok(summary) => summary,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping window subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("window command sequence failed: {reason}");
            }
        },
        Err(_) => panic!("window command thread should exit cleanly"),
    };

    let events = match subscription_thread.join() {
        Ok(events) => match events {
            Ok(events) => events,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping window subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("window subscription failed: {reason}");
            }
        },
        Err(_) => panic!("window subscription thread should exit cleanly"),
    };

    assert_eq!(events.geometry.surface_id, TARGET_SURFACE_ID);
    assert_eq!(events.geometry.previous_x, summary.initial_window.x);
    assert_eq!(events.geometry.previous_y, summary.initial_window.y);
    assert_eq!(events.geometry.previous_width, summary.initial_window.width);
    assert_eq!(events.geometry.previous_height, summary.initial_window.height);
    assert_eq!(events.geometry.x, summary.target_x);
    assert_eq!(events.geometry.y, summary.target_y);
    assert_eq!(events.geometry.width, summary.initial_window.width);
    assert_eq!(events.geometry.height, summary.initial_window.height);

    assert_eq!(events.state.surface_id, TARGET_SURFACE_ID);
    assert_eq!(events.state.previous_state, summary.initial_window.state);
    assert_eq!(events.state.state, "Floating");
}

/// Seeds one window for the subscription scenario.
fn seed_window(world: &mut bevy_ecs::world::World) {
    world.spawn((WindowBundle {
        surface: WlSurfaceHandle { id: TARGET_SURFACE_ID },
        geometry: SurfaceGeometry { x: 0, y: 32, width: 440, height: 700 },
        window: XdgWindow {
            app_id: "org.nekoland.window-subscription".to_owned(),
            title: "Subscription Window".to_owned(),
            last_acked_configure: None,
        },
        layout: WindowLayout::Tiled,
        mode: WindowMode::Normal,
        ..Default::default()
    },));
}

/// Returns the default config path used by this integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

/// Waits until the seeded window appears in the tree query, then issues the move request.
fn issue_move_command_when_window_is_ready(
    socket_path: &Path,
) -> Result<WindowChangeCommandSummary, common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        match query_tree(socket_path) {
            Ok(tree) => {
                let Some(window) =
                    tree.windows.into_iter().find(|window| window.surface_id == TARGET_SURFACE_ID)
                else {
                    if Instant::now() >= deadline {
                        return Err(common::TestControl::Fail(
                            "timed out waiting for window before issuing move request".to_owned(),
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };

                let target_x = window.x + 137;
                let target_y = window.y + 53;
                let reply = send_request_to_path(
                    socket_path,
                    &IpcRequest {
                        correlation_id: 7,
                        command: IpcCommand::Window(WindowCommand::Move {
                            surface_id: TARGET_SURFACE_ID,
                            x: i64::from(target_x),
                            y: i64::from(target_y),
                        }),
                    },
                )
                .map_err(classify_ipc_error)?;

                if !reply.ok {
                    return Err(common::TestControl::Fail(format!(
                        "IPC move request was rejected: {reply:?}"
                    )));
                }

                return Ok(WindowChangeCommandSummary {
                    initial_window: window,
                    target_x,
                    target_y,
                });
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) => return Err(classify_ipc_error(error)),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for window before issuing move request".to_owned(),
            ));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits for both geometry-change and state-change events targeting the expected window.
fn wait_for_window_change_events(
    socket_path: &Path,
    subscription: IpcSubscription,
    expected_surface: u64,
) -> Result<WindowChangeEvents, common::TestControl> {
    let mut stream = subscribe_to_path(socket_path, &subscription).map_err(classify_ipc_error)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut geometry = None;
    let mut state = None;

    loop {
        match stream.read_event() {
            Ok(event) => {
                let Some(payload) = event.payload else {
                    continue;
                };

                // Geometry and state transitions may be delivered as separate
                // events, so cache each half until both have arrived.
                match event.event.as_str() {
                    "window_geometry_changed" => {
                        let change =
                            serde_json::from_value::<WindowGeometryChangeSnapshot>(payload)
                                .map_err(|error| {
                                    common::TestControl::Fail(format!(
                                        "failed to decode window_geometry_changed payload: {error}"
                                    ))
                                })?;
                        if change.surface_id == expected_surface {
                            geometry = Some(change);
                        }
                    }
                    "window_state_changed" => {
                        let change = serde_json::from_value::<WindowStateChangeSnapshot>(payload)
                            .map_err(|error| {
                            common::TestControl::Fail(format!(
                                "failed to decode window_state_changed payload: {error}"
                            ))
                        })?;
                        if change.surface_id == expected_surface {
                            state = Some(change);
                        }
                    }
                    _ => {}
                }

                if let (Some(geometry), Some(state)) = (geometry.clone(), state.clone()) {
                    return Ok(WindowChangeEvents { geometry, state });
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) => return Err(classify_ipc_error(error)),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for window change subscription events".to_owned(),
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
    )
}

/// Identifies IPC errors that should skip the test in restricted environments.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}
