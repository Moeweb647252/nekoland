use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::{ResMut, Resource};
use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_core::schedules::ExtractSchedule;
use nekoland_ecs::resources::{BackendInputAction, BackendInputEvent, PendingBackendInputEvents};
use nekoland_ipc::commands::{CommandSnapshot, QueryCommand};
use nekoland_ipc::{
    IpcCommand, IpcReply, IpcRequest, IpcServerState, IpcSubscription, IpcSubscriptionEvent,
    SubscriptionTopic, send_request_to_path, subscribe_to_path,
};

mod common;

const SUPER_KEYCODE: u32 = 133;
const SPACE_KEYCODE: u32 = 65;
const TEST_CONFIG: &str = r##"
default_layout = "tiling"

[theme]
name = "catppuccin-latte"
cursor_theme = "default"
border_color = "#5c7cfa"
background_color = "#f5f7ff"

[input]
focus_follows_mouse = true
repeat_rate = 30

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Space" = "exec /definitely-not-a-real-nekoland-command"
"##;

#[derive(Debug, Default, Resource)]
struct CommandInputPump {
    injected: bool,
}

#[test]
fn command_subscription_reports_failed_external_command_invocations() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-command-events");
    let config_path = runtime_dir.path.join("command-events.toml");
    fs::write(&config_path, TEST_CONFIG).expect("command-events config should be writable");

    let mut app = build_app(&config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(80),
    });
    app.inner_mut()
        .init_resource::<CommandInputPump>()
        .add_systems(ExtractSchedule, inject_command_keybinding_input);

    let ipc_socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<IpcServerState>()
            .expect("IPC server state should be available immediately after build");

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping command subscription test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let test_thread = thread::spawn(move || {
        let event = collect_command_failure_event(
            &ipc_socket_path,
            IpcSubscription {
                topic: SubscriptionTopic::Command,
                include_payloads: true,
                events: vec!["command_failed".to_owned()],
            },
        )?;
        let commands = wait_for_command_history(&ipc_socket_path)?;
        Ok::<_, common::TestControl>((event, commands))
    });
    app.run().expect("nekoland app should complete the configured frame budget");

    let (event, commands) =
        match test_thread.join().expect("command test thread should exit cleanly") {
            Ok(result) => result,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping command subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("command subscription test failed: {reason}");
            }
        };

    assert_eq!(event.topic, SubscriptionTopic::Command);
    assert_eq!(event.event, "command_failed");

    let payload = event.payload.expect("command failure subscription should carry a payload");
    assert_eq!(
        payload["origin"].as_str(),
        Some("Super+Space -> exec /definitely-not-a-real-nekoland-command")
    );
    assert_eq!(
        payload["candidates"][0][0].as_str(),
        Some("/definitely-not-a-real-nekoland-command")
    );

    let error = payload["error"].as_str().expect("failure payload should include an error");
    assert!(
        error.contains("No such file") || error.contains("not found"),
        "spawn failure should surface the OS error message: {error}"
    );

    assert!(
        !commands.is_empty(),
        "command history query should report at least one command record"
    );
    let latest = commands.last().expect("non-empty command history should have a last entry");
    assert_eq!(latest.origin, "Super+Space -> exec /definitely-not-a-real-nekoland-command");
    assert!(
        latest.command.is_none(),
        "failed command history entries should not claim a resolved argv"
    );
    assert_eq!(latest.candidates, vec![vec!["/definitely-not-a-real-nekoland-command".to_owned()]]);
    assert!(
        matches!(
            latest.status.as_ref(),
            Some(nekoland_ipc::commands::CommandStatusSnapshot::Failed { error })
                if error.contains("No such file") || error.contains("not found")
        ),
        "command history should preserve the failure status: {latest:?}"
    );
}

fn inject_command_keybinding_input(
    mut pump: ResMut<CommandInputPump>,
    mut pending_backend_inputs: ResMut<PendingBackendInputEvents>,
) {
    if pump.injected {
        return;
    }

    pending_backend_inputs.items.extend([
        BackendInputEvent {
            device: "command-events-test".to_owned(),
            action: BackendInputAction::Key { keycode: SUPER_KEYCODE, pressed: true },
        },
        BackendInputEvent {
            device: "command-events-test".to_owned(),
            action: BackendInputAction::Key { keycode: SPACE_KEYCODE, pressed: true },
        },
    ]);
    pump.injected = true;
}

fn collect_command_failure_event(
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
            Ok(event) => return Ok(event),
            Err(error) if ipc_error_is_retryable(&error) => {
                if Instant::now() >= deadline {
                    return Err(common::TestControl::Fail(
                        "timed out waiting for a command_failed subscription event".to_owned(),
                    ));
                }
            }
            Err(error) if ipc_error_is_skippable(&error) => {
                return Err(common::TestControl::Skip(error.to_string()));
            }
            Err(error) => return Err(common::TestControl::Fail(error.to_string())),
        }
    }
}

fn wait_for_command_history(
    socket_path: &Path,
) -> Result<Vec<CommandSnapshot>, common::TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        let request =
            IpcRequest { correlation_id: 2, command: IpcCommand::Query(QueryCommand::GetCommands) };
        match send_request_to_path(socket_path, &request) {
            Ok(reply) => {
                let commands = decode_command_history_reply(reply)?;
                if !commands.is_empty() {
                    return Ok(commands);
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
                "timed out waiting for command history to become queryable".to_owned(),
            ));
        }
    }
}

fn decode_command_history_reply(
    reply: IpcReply,
) -> Result<Vec<CommandSnapshot>, common::TestControl> {
    if !reply.ok {
        return Err(common::TestControl::Fail(format!(
            "command history query failed: {}",
            reply.message
        )));
    }

    let payload = reply
        .payload
        .ok_or_else(|| common::TestControl::Fail("command query returned no payload".to_owned()))?;
    serde_json::from_value(payload).map_err(|error| {
        common::TestControl::Fail(format!("invalid command query payload: {error}"))
    })
}

fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::WouldBlock | ErrorKind::TimedOut
    ) || error.raw_os_error() == Some(1)
}
