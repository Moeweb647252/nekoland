use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ipc::commands::WorkspaceCommand;
use nekoland_ipc::{
    IpcCommand, IpcRequest, IpcServerState, IpcSubscriptionEvent, send_request_to_path,
};
use serde_json::Value;

#[test]
fn subscribe_help_lists_topics_and_known_event_names() {
    let output = Command::new(env!("CARGO_BIN_EXE_nekoland-msg"))
        .arg("subscribe")
        .arg("--help")
        .output()
        .expect("subscribe help should execute");

    let stdout = String::from_utf8(output.stdout).expect("subscribe help stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("subscribe help stderr should be UTF-8");

    assert!(
        output.status.success(),
        "subscribe help should exit successfully\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("Topics:"),
        "subscribe help should list supported topics\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("window")
            && stdout.contains("workspace")
            && stdout.contains("command")
            && stdout.contains("config")
            && stdout.contains("clipboard")
            && stdout.contains("primary-selection")
            && stdout.contains("focus"),
        "subscribe help should include concrete topic names\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("Known events:")
            && stdout.contains("window_created")
            && stdout.contains("window_geometry_changed")
            && stdout.contains("window_state_changed")
            && stdout.contains("popup_geometry_changed")
            && stdout.contains("popup_grab_changed")
            && stdout.contains("command_failed")
            && stdout.contains("config_changed")
            && stdout.contains("clipboard_changed")
            && stdout.contains("primary_selection_changed")
            && stdout.contains("focus_changed"),
        "subscribe help should list known event names\nstdout:\n{stdout}"
    );
}

#[test]
fn subscribe_help_supports_machine_readable_json_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_nekoland-msg"))
        .arg("subscribe")
        .arg("--help")
        .arg("--json")
        .output()
        .expect("subscribe json help should execute");

    let stdout =
        String::from_utf8(output.stdout).expect("subscribe json help stdout should be UTF-8");
    let stderr =
        String::from_utf8(output.stderr).expect("subscribe json help stderr should be UTF-8");

    assert!(
        output.status.success(),
        "subscribe json help should exit successfully\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let help =
        serde_json::from_str::<Value>(&stdout).expect("subscribe json help should be valid JSON");
    assert_eq!(help["topics"][0], "window");
    assert!(
        help["topics"]
            .as_array()
            .expect("topics should be an array")
            .iter()
            .any(|topic| topic == "command")
    );
    assert!(
        help["topics"]
            .as_array()
            .expect("topics should be an array")
            .iter()
            .any(|topic| topic == "config")
    );
    assert!(
        help["topics"]
            .as_array()
            .expect("topics should be an array")
            .iter()
            .any(|topic| topic == "clipboard")
    );
    assert!(
        help["topics"]
            .as_array()
            .expect("topics should be an array")
            .iter()
            .any(|topic| topic == "primary-selection")
    );
    assert!(
        help["topics"]
            .as_array()
            .expect("topics should be an array")
            .iter()
            .any(|topic| topic == "focus")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "window_geometry_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "window_state_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "popup_geometry_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "popup_grab_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "command_failed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "config_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "clipboard_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "primary_selection_changed")
    );
    assert!(
        help["known_events"]
            .as_array()
            .expect("known_events should be an array")
            .iter()
            .any(|event| event == "focus_changed")
    );
    assert_eq!(help["patterns"]["prefix_wildcard_example"], "window_*");
}

#[test]
fn completion_subcommand_generates_bash_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_nekoland-msg"))
        .arg("completion")
        .arg("bash")
        .output()
        .expect("completion bash should execute");

    let stdout = String::from_utf8(output.stdout).expect("completion bash stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("completion bash stderr should be UTF-8");

    assert!(
        output.status.success(),
        "completion bash should exit successfully\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("nekoland-msg"));
    assert!(stdout.contains("subscribe"));
}

#[test]
fn subscribe_cli_streams_workspace_events_from_ipc() {
    let _env_lock = env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = RuntimeDirGuard::new("nekoland-msg-subscribe-cli");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(160),
    });

    let ipc_socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<IpcServerState>()
            .expect("IPC server state should be available immediately after build");

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping subscribe CLI test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let child = Command::new(env!("CARGO_BIN_EXE_nekoland-msg"))
        .arg("subscribe")
        .arg("workspace")
        .arg("--no-payloads")
        .env("NEKOLAND_RUNTIME_DIR", &runtime_dir.path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("subscribe CLI should spawn");

    let ipc_thread = thread::spawn(move || emit_workspace_events(&ipc_socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let ipc_result = ipc_thread.join().expect("IPC control thread should exit cleanly");
    if let Err(reason) = ipc_result {
        panic!("workspace event emission failed: {reason}");
    }

    drop(app);
    let output = child.wait_with_output().expect("subscribe CLI should exit cleanly");
    let stdout = String::from_utf8(output.stdout).expect("subscribe CLI stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("subscribe CLI stderr should be UTF-8");

    assert!(
        output.status.success(),
        "subscribe CLI should exit successfully after IPC shutdown\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("\"topic\": \"Workspace\""),
        "subscribe CLI should print workspace topic events\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("\"event\": \"workspaces_changed\""),
        "subscribe CLI should print workspace change events\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("\"payload\": null"),
        "subscribe CLI should honor --no-payloads and omit payload bodies\nstdout:\n{stdout}"
    );
}

#[test]
fn subscribe_cli_supports_jsonl_output_for_scripts() {
    let _env_lock = env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = RuntimeDirGuard::new("nekoland-msg-subscribe-jsonl-cli");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(160),
    });

    let ipc_socket_path = {
        let world = app.inner().world();
        let server_state = world
            .get_resource::<IpcServerState>()
            .expect("IPC server state should be available immediately after build");

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping subscribe CLI jsonl test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let child = Command::new(env!("CARGO_BIN_EXE_nekoland-msg"))
        .arg("subscribe")
        .arg("all")
        .arg("--event")
        .arg("workspaces_*")
        .arg("--event")
        .arg("tree_*")
        .arg("--jsonl")
        .arg("--no-payloads")
        .env("NEKOLAND_RUNTIME_DIR", &runtime_dir.path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("subscribe CLI should spawn");

    let ipc_thread = thread::spawn(move || emit_workspace_events(&ipc_socket_path));
    app.run().expect("nekoland app should complete the configured frame budget");

    let ipc_result = ipc_thread.join().expect("IPC control thread should exit cleanly");
    if let Err(reason) = ipc_result {
        panic!("workspace event emission failed: {reason}");
    }

    drop(app);
    let output = child.wait_with_output().expect("subscribe CLI should exit cleanly");
    let stdout = String::from_utf8(output.stdout).expect("subscribe CLI stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("subscribe CLI stderr should be UTF-8");

    assert!(
        output.status.success(),
        "subscribe CLI should exit successfully after IPC shutdown\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let lines = stdout.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>();
    assert!(
        !lines.is_empty(),
        "jsonl subscribe mode should emit at least one event line\nstdout:\n{stdout}"
    );

    let events = lines
        .iter()
        .map(|line| {
            serde_json::from_str::<IpcSubscriptionEvent>(line)
                .expect("each jsonl line should decode as an IPC subscription event")
        })
        .collect::<Vec<_>>();
    assert!(
        events.iter().any(|event| event.event == "workspaces_changed"),
        "jsonl subscribe mode should include workspace change events: {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| matches!(event.event.as_str(), "workspaces_changed" | "tree_changed")),
        "server-side event filter should suppress non-matching events in jsonl mode: {events:?}"
    );
    assert!(
        events.iter().all(|event| event.payload.is_none()),
        "jsonl subscribe mode should omit payload bodies when --no-payloads is set: {events:?}"
    );
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug)]
struct RuntimeDirGuard {
    previous: Option<OsString>,
    path: PathBuf,
}

impl RuntimeDirGuard {
    fn new(prefix: &str) -> Self {
        let path = temporary_runtime_dir(prefix);
        fs::create_dir_all(&path).expect("test runtime dir should be creatable");
        let previous = std::env::var_os("NEKOLAND_RUNTIME_DIR");

        unsafe {
            std::env::set_var("NEKOLAND_RUNTIME_DIR", &path);
        }

        Self { previous, path }
    }
}

impl Drop for RuntimeDirGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => unsafe {
                std::env::set_var("NEKOLAND_RUNTIME_DIR", previous);
            },
            None => unsafe {
                std::env::remove_var("NEKOLAND_RUNTIME_DIR");
            },
        }

        let _ = fs::remove_dir_all(&self.path);
    }
}

fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}

fn temporary_runtime_dir(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after the unix epoch")
        .as_nanos();
    path.push(format!("{prefix}-{}-{unique}", std::process::id()));
    path
}

fn emit_workspace_events(socket_path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(250);
    let create_request = IpcRequest {
        correlation_id: 1,
        command: IpcCommand::Workspace(WorkspaceCommand::Create { workspace: "2".to_owned() }),
    };
    let activate_request = IpcRequest {
        correlation_id: 2,
        command: IpcCommand::Workspace(WorkspaceCommand::Switch { workspace: "2".to_owned() }),
    };
    let deactivate_request = IpcRequest {
        correlation_id: 3,
        command: IpcCommand::Workspace(WorkspaceCommand::Switch { workspace: "1".to_owned() }),
    };

    while Instant::now() < deadline {
        let _ = send_request_to_path(socket_path, &create_request);
        let _ = send_request_to_path(socket_path, &activate_request);
        let _ = send_request_to_path(socket_path, &deactivate_request);
        thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}
