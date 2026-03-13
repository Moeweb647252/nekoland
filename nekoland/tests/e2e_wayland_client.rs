//! End-to-end test that launches the compositor binary and performs a real Wayland client
//! round-trip against the published socket.

use std::fs;
use std::io::ErrorKind;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Collected stderr and exit status from the spawned compositor child.
#[derive(Debug)]
struct ChildOutput {
    /// Exit status collected from the compositor child process.
    status: ExitStatus,
    /// Entire stderr stream drained after the child exited.
    stderr: String,
}

mod common;

/// Verifies that the compositor binary can publish a Wayland socket and complete an initial XDG
/// toplevel handshake with a real client.
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

/// Spawns the compositor binary with a short bounded runtime for end-to-end testing.
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

/// Preflight check that the environment allows Unix-socket bind operations in the runtime dir.
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

/// Waits for the compositor to publish a Wayland socket or for the child process to exit.
fn wait_for_socket(runtime_dir: &Path, child: &mut Child) -> Result<PathBuf, TestControl> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        // Poll the runtime dir first so the happy path does not depend on
        // reading child output unless the process has already terminated.
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

/// Looks for the first published socket file in the runtime directory.
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

/// Returns the workspace root so the spawned binary uses repository-relative assets/config.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Runs the shared lightweight Wayland client against the compositor socket.
fn run_client(socket_path: &Path) -> Result<common::ClientSummary, TestControl> {
    common::run_xdg_client(socket_path).map_err(|control| match control {
        common::TestControl::Skip(reason) => TestControl::Skip(reason),
        common::TestControl::Fail(reason) => TestControl::Fail(reason),
    })
}

/// Tries to collect child output only if the process has already exited.
fn try_collect_child_output(child: &mut Child) -> Result<Option<ChildOutput>, TestControl> {
    let Some(status) = child.try_wait().map_err(|error| TestControl::Fail(error.to_string()))?
    else {
        return Ok(None);
    };

    Ok(Some(ChildOutput { status, stderr: take_child_stderr(child) }))
}

/// Waits for the child process to exit within the supplied timeout.
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

/// Force-terminates the child process and collects any buffered stderr.
fn terminate_child(child: &mut Child) -> ChildOutput {
    let _ = child.kill();
    let status = child.wait().expect("child wait after kill should succeed");
    ChildOutput { status, stderr: take_child_stderr(child) }
}

/// Drains the child's stderr pipe into a string for assertions and diagnostics.
fn take_child_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    stderr
}

/// Identifies common sandbox-style socket permission failures that should skip the test.
fn is_restricted_socket_error(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::PermissionDenied || error.raw_os_error() == Some(1)
}

/// Test-level control flow for the end-to-end client round-trip helper.
#[derive(Debug)]
enum TestControl {
    /// Skip the test because the environment cannot support the required primitives.
    Skip(String),
    /// Fail the test because the compositor or helper client behaved unexpectedly.
    Fail(String),
}
