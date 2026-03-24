//! In-process integration test for present-audit IPC query and subscription state.

use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    SurfaceGeometry, WindowLayout, WindowMode, WlSurfaceHandle, XdgWindow,
};
use nekoland_ipc::commands::{PresentAuditOutputSnapshot, QueryCommand};
use nekoland_ipc::{
    IpcCommand, IpcReply, IpcRequest, IpcServerState, IpcSubscription, IpcSubscriptionEvent,
    SubscriptionTopic, send_request_to_path, subscribe_to_path,
};

mod common;

/// Surface id of the seeded floating window that should appear in the present audit.
const TEST_SURFACE_ID: u64 = 4242;

/// Verifies that present-audit state is visible through both IPC subscription events and the IPC
/// query snapshot.
#[test]
fn ipc_reports_present_audit_query_and_subscription_updates() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-ipc-present-audit");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "ipc-present-audit.toml",
    );

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(128),
    });
    seed_floating_window(app.inner_mut().world_mut());

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(ipc_server_state) = world.get_resource::<IpcServerState>() else {
            panic!("ipc server state should be available immediately after build");
        };

        match (ipc_server_state.listening, &ipc_server_state.startup_error) {
            (true, _) => ipc_server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping IPC present-audit test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let ipc_thread = thread::spawn(move || {
        let event = wait_for_present_audit_changed(
            &ipc_socket_path,
            IpcSubscription {
                topic: SubscriptionTopic::PresentAudit,
                include_payloads: true,
                events: vec!["present_audit_changed".to_owned()],
            },
        )?;
        let snapshot = wait_for_present_audit_query(&ipc_socket_path)?;
        Ok::<_, common::TestControl>((event, snapshot))
    });

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    let (event, snapshot) = match ipc_thread.join() {
        Ok(result) => match result {
            Ok(result) => result,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping IPC present-audit test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("present-audit IPC test failed: {reason}")
            }
        },
        Err(_) => panic!("ipc thread should exit cleanly"),
    };

    assert_eq!(event.topic, SubscriptionTopic::PresentAudit);
    assert_eq!(event.event, "present_audit_changed");

    let Some(payload) = event.payload else {
        panic!("present_audit_changed should include a payload");
    };
    let Ok(event_snapshot) = serde_json::from_value::<Vec<PresentAuditOutputSnapshot>>(payload)
    else {
        panic!("present_audit_changed payload should decode");
    };
    assert_present_audit_snapshot(&event_snapshot);
    assert_present_audit_snapshot(&snapshot);
}

/// Seeds one floating window so present-audit has deterministic content to report.
fn seed_floating_window(world: &mut bevy_ecs::world::World) {
    world.spawn((WindowBundle {
        surface: WlSurfaceHandle { id: TEST_SURFACE_ID },
        geometry: SurfaceGeometry { x: 64, y: 48, width: 400, height: 240 },
        window: XdgWindow {
            app_id: "org.nekoland.present-audit".to_owned(),
            title: "Present Audit Window".to_owned(),
        },
        layout: WindowLayout::Floating,
        mode: WindowMode::Normal,
        ..Default::default()
    },));
}

/// Waits for the first present-audit subscription event that contains the seeded window.
fn wait_for_present_audit_changed(
    socket_path: &Path,
    subscription: IpcSubscription,
) -> Result<IpcSubscriptionEvent, common::TestControl> {
    let mut stream = subscribe_to_path(socket_path, &subscription).map_err(|error| {
        if ipc_error_is_skippable(&error) {
            common::TestControl::Skip(error.to_string())
        } else {
            common::TestControl::Fail(error.to_string())
        }
    })?;

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match stream.read_event() {
            Ok(event) => {
                let Some(payload) = event.payload.clone() else {
                    continue;
                };
                let snapshots = serde_json::from_value::<Vec<PresentAuditOutputSnapshot>>(payload)
                    .map_err(|error| {
                        common::TestControl::Fail(format!(
                            "present_audit_changed payload should decode: {error}"
                        ))
                    })?;
                if present_audit_has_seeded_window(&snapshots) {
                    return Ok(event);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {
                if Instant::now() >= deadline {
                    let snapshot = send_request_to_path(
                        socket_path,
                        &IpcRequest {
                            correlation_id: 99,
                            command: IpcCommand::Query(QueryCommand::GetPresentAudit),
                        },
                    )
                    .ok()
                    .and_then(|reply| decode_present_audit_reply(reply).ok());
                    return Err(common::TestControl::Fail(format!(
                        "timed out waiting for present_audit_changed (latest_query={snapshot:?})"
                    )));
                }
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => return Err(common::TestControl::Fail(error.to_string())),
        }
    }
}

/// Polls the IPC present-audit query until it returns the seeded window audit snapshot.
fn wait_for_present_audit_query(
    socket_path: &Path,
) -> Result<Vec<PresentAuditOutputSnapshot>, common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        let request = IpcRequest {
            correlation_id: 2,
            command: IpcCommand::Query(QueryCommand::GetPresentAudit),
        };
        match send_request_to_path(socket_path, &request) {
            Ok(reply) => {
                let snapshot = decode_present_audit_reply(reply)?;
                if present_audit_has_seeded_window(&snapshot) {
                    return Ok(snapshot);
                }
            }
            Err(error) if ipc_error_is_retryable(&error) => {}
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => return Err(common::TestControl::Fail(error.to_string())),
        }

        if Instant::now() >= deadline {
            return Err(common::TestControl::Fail(
                "timed out waiting for present-audit query to expose seeded window".to_owned(),
            ));
        }
    }
}

/// Decodes the present-audit query reply payload into output-local audit snapshots.
fn decode_present_audit_reply(
    reply: IpcReply,
) -> Result<Vec<PresentAuditOutputSnapshot>, common::TestControl> {
    if !reply.ok {
        return Err(common::TestControl::Fail(format!(
            "present-audit query failed: {}",
            reply.message
        )));
    }

    let payload = reply.payload.ok_or_else(|| {
        common::TestControl::Fail("present-audit query returned no payload".to_owned())
    })?;
    serde_json::from_value(payload).map_err(|error| {
        common::TestControl::Fail(format!("invalid present-audit query payload: {error}"))
    })
}

fn present_audit_has_seeded_window(snapshots: &[PresentAuditOutputSnapshot]) -> bool {
    snapshots.iter().any(|output| {
        output.elements.iter().any(|element| {
            element.surface_id == TEST_SURFACE_ID
                && element.kind == "window"
                && (element.x, element.y, element.width, element.height) == (64, 48, 400, 240)
        })
    })
}

fn assert_present_audit_snapshot(snapshots: &[PresentAuditOutputSnapshot]) {
    assert!(
        present_audit_has_seeded_window(snapshots),
        "present-audit snapshot should expose the seeded floating window: {snapshots:?}"
    );
}

/// Identifies retryable transient IPC errors.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Identifies IPC errors that should skip the test in restricted environments.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::WouldBlock | ErrorKind::TimedOut
    ) || error.raw_os_error() == Some(1)
}
