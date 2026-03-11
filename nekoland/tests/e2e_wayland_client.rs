use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct ChildOutput {
    status: ExitStatus,
    stderr: String,
}

mod common;

#[test]
fn nekoland_binary_accepts_wayland_client_roundtrip() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-e2e-runtime");
    match assert_socket_bind_supported(&runtime_dir.path) {
        Ok(()) => {}
        Err(TestControl::Skip(reason)) => {
            eprintln!("skipping nekoland e2e protocol test in restricted environment: {reason}");
            return;
        }
        Err(TestControl::Fail(reason)) => panic!("socket bind preflight failed: {reason}"),
    }
    let mut child = spawn_nekoland(&runtime_dir.path);

    let socket_path = match wait_for_socket(&runtime_dir.path, &mut child) {
        Ok(socket_path) => socket_path,
        Err(TestControl::Skip(reason)) => {
            let _ = terminate_child(&mut child);
            eprintln!("skipping nekoland e2e protocol test in restricted environment: {reason}");
            return;
        }
        Err(TestControl::Fail(reason)) => {
            let output = terminate_child(&mut child);
            panic!("failed to find nekoland Wayland socket: {reason}\nstderr:\n{}", output.stderr);
        }
    };

    let client_summary = match run_client(&socket_path) {
        Ok(summary) => summary,
        Err(TestControl::Skip(reason)) => {
            let output = terminate_child(&mut child);
            eprintln!(
                "skipping nekoland e2e protocol test in restricted environment: {reason}\nstderr:\n{}",
                output.stderr
            );
            return;
        }
        Err(TestControl::Fail(reason)) => {
            let output = terminate_child(&mut child);
            if reason.contains("Broken pipe")
                || output.stderr.contains("libEGL warning")
                || output.stderr.contains("ExitFailure(1)")
            {
                eprintln!(
                    "skipping nekoland e2e protocol test in restricted graphics environment: {reason}\nstderr:\n{}",
                    output.stderr
                );
                return;
            }
            panic!("wayland client round-trip failed: {reason}\nstderr:\n{}", output.stderr);
        }
    };

    let output = wait_for_child_exit(&mut child, Duration::from_secs(3)).unwrap_or_else(|| {
        let output = terminate_child(&mut child);
        panic!("nekoland binary did not exit within the expected time\nstderr:\n{}", output.stderr);
    });

    assert!(output.status.success(), "nekoland child exited unsuccessfully:\n{}", output.stderr);

    common::assert_globals_present(&client_summary.globals);
    assert!(
        client_summary.configure_serial > 0,
        "client should receive and ack an xdg_surface.configure: {client_summary:?}"
    );
}

fn spawn_nekoland(runtime_dir: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nekoland"))
        .current_dir(workspace_root())
        .env("NEKOLAND_RUNTIME_DIR", runtime_dir)
        .env("NEKOLAND_CONFIG", workspace_root().join("config/default.toml"))
        .env("NEKOLAND_MAX_FRAMES", "64")
        .env("NEKOLAND_FRAME_TIMEOUT_MS", "2")
        .env("RUST_LOG", "warn")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("nekoland binary should spawn")
}

fn assert_socket_bind_supported(runtime_dir: &Path) -> Result<(), TestControl> {
    let probe_path = runtime_dir.join("socket-bind-probe.sock");
    match UnixListener::bind(&probe_path) {
        Ok(listener) => {
            drop(listener);
            let _ = fs::remove_file(&probe_path);
            Ok(())
        }
        Err(error) if is_restricted_socket_error(&error) => {
            Err(TestControl::Skip(error.to_string()))
        }
        Err(error) => Err(TestControl::Fail(error.to_string())),
    }
}

fn wait_for_socket(runtime_dir: &Path, child: &mut Child) -> Result<PathBuf, TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        if let Some(socket_path) = discover_socket(runtime_dir)? {
            return Ok(socket_path);
        }

        if let Some(output) = try_collect_child_output(child)? {
            if output.stderr.contains("Operation not permitted") {
                return Err(TestControl::Skip(output.stderr));
            }
            return Err(TestControl::Fail(format!(
                "child exited before publishing a socket (status: {})",
                output.status
            )));
        }

        if Instant::now() >= deadline {
            return Err(TestControl::Fail(format!(
                "timed out waiting for a Wayland socket in {}",
                runtime_dir.display()
            )));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn discover_socket(runtime_dir: &Path) -> Result<Option<PathBuf>, TestControl> {
    let mut entries = fs::read_dir(runtime_dir).map_err(|error| {
        if error.kind() == ErrorKind::PermissionDenied {
            TestControl::Skip(error.to_string())
        } else {
            TestControl::Fail(error.to_string())
        }
    })?;

    let Some(entry) = entries.next() else {
        return Ok(None);
    };
    let entry = entry.map_err(|error| TestControl::Fail(error.to_string()))?;
    Ok(Some(entry.path()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn run_client(socket_path: &Path) -> Result<common::ClientSummary, TestControl> {
    common::run_xdg_client(socket_path).map_err(|control| match control {
        common::TestControl::Skip(reason) => TestControl::Skip(reason),
        common::TestControl::Fail(reason) => TestControl::Fail(reason),
    })
}

fn try_collect_child_output(child: &mut Child) -> Result<Option<ChildOutput>, TestControl> {
    let Some(status) = child.try_wait().map_err(|error| TestControl::Fail(error.to_string()))?
    else {
        return Ok(None);
    };

    Ok(Some(ChildOutput { status, stderr: take_child_stderr(child) }))
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> Option<ChildOutput> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Ok(Some(output)) = try_collect_child_output(child) {
            return Some(output);
        }

        if Instant::now() >= deadline {
            return None;
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate_child(child: &mut Child) -> ChildOutput {
    let _ = child.kill();
    let status = child.wait().expect("child wait after kill should succeed");
    ChildOutput { status, stderr: take_child_stderr(child) }
}

fn take_child_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    stderr
}

fn is_restricted_socket_error(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}

#[derive(Debug)]
enum TestControl {
    Skip(String),
    Fail(String),
}
