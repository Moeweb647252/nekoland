//! Startup-level integration test that verifies the compositor publishes the expected protocol
//! globals and core runtime resources after boot.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use nekoland::build_app;
use nekoland_backend::BackendOutputRegistry;
use nekoland_core::app::RunLoopSettings;
use nekoland_ipc::IpcServerState;
use nekoland_protocol::{
    ProtocolRegistry, ProtocolServerState, XWaylandServerState, supported_protocols,
};

mod common;

/// Verifies that a short compositor startup run populates the protocol registry, protocol server
/// state, seeded outputs, and IPC server state.
#[test]
fn startup_registers_protocol_globals_and_runtime_state() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-runtime");
    let config_path = workspace_config_path();

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(0),
        max_frames: Some(1),
    });

    if let Err(error) = app.run() {
        panic!("nekoland should start and complete one frame: {error}");
    }

    let world = app.inner().world();
    let Some(registry) = world.get_resource::<ProtocolRegistry>() else {
        panic!("protocol registry should be inserted during startup");
    };
    let Some(server_state) = world.get_resource::<ProtocolServerState>() else {
        panic!("protocol server state should be inserted during startup");
    };
    let Some(outputs) = world.get_resource::<BackendOutputRegistry>() else {
        panic!("backend output registry should be inserted during startup");
    };
    let Some(xwayland) = world.get_resource::<XWaylandServerState>() else {
        panic!("xwayland server state should be inserted during startup");
    };
    let Some(ipc_server_state) = world.get_resource::<IpcServerState>() else {
        panic!("IPC server state should be inserted during startup");
    };

    let actual_globals = registry.globals.iter().copied().collect::<BTreeSet<_>>();
    let expected_globals = supported_protocols().iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(actual_globals, expected_globals, "startup should expose the expected globals");
    assert!(
        server_state.socket_name.is_some() || server_state.startup_error.is_some(),
        "startup should either publish a Wayland socket or record why binding failed: {server_state:?}"
    );
    assert!(
        outputs.connected_by_id.values().any(|name| !name.is_empty()),
        "startup should seed at least one output: {outputs:?}"
    );
    assert!(
        ipc_server_state.listening || ipc_server_state.startup_error.is_some(),
        "startup should either publish an IPC socket or record why binding failed: {ipc_server_state:?}"
    );
    assert!(
        xwayland.enabled || xwayland.startup_error.is_some() || !xwayland.ready,
        "xwayland should either start, remain pending, or explain why startup failed: {xwayland:?}"
    );

    drop(runtime_dir);
}

/// Returns the default config path used by this startup integration test.
fn workspace_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/default.toml")
}
