//! In-process integration test for the `config_changed` subscription stream.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::resources::ConfiguredAction;
use nekoland_ipc::commands::ConfigSnapshot;
use nekoland_ipc::{
    IpcServerState, IpcSubscription, IpcSubscriptionEvent, SubscriptionTopic, subscribe_to_path,
};

mod common;

/// Baseline config served before the subscription observes a reload.
const INITIAL_CONFIG: &str = r##"
default_layout = "tiling"

[theme]
name = "latte"
cursor_theme = "breeze"
border_color = "#112233"
background_color = "#f5f5f5"

[input]
focus_follows_mouse = false
repeat_rate = 30

[input.keyboard]
current = "us"

[[input.keyboard.layouts]]
name = "us"
layout = "us"

[ipc]
command_history_limit = 7

[xwayland]
enabled = true

[[outputs]]
name = "eDP-1"
mode = "1920x1080@60"
scale = 1
enabled = true

[keybinds.bindings]
"Super+Alt" = { viewport_pan_mode = true }
"Super+Return" = { exec = ["foot"] }
"##;

/// Replacement config written while the compositor is already serving IPC.
const RELOADED_CONFIG: &str = r##"
default_layout = "floating"

[theme]
name = "frappe"
cursor_theme = "capitaine"
border_color = "#445566"
background_color = "#101010"

[input]
focus_follows_mouse = true
repeat_rate = 45

[input.keyboard]
current = "de"

[[input.keyboard.layouts]]
name = "us"
layout = "us"

[[input.keyboard.layouts]]
name = "de"
layout = "de"
variant = "nodeadkeys"

[ipc]
command_history_limit = 3

[xwayland]
enabled = false

[[outputs]]
name = "HDMI-A-1"
mode = "2560x1440@75"
scale = 2
enabled = true

[keybinds.bindings]
"Ctrl+Shift" = { viewport_pan_mode = true }
"Super+P" = { exec = ["wlogout", "--protocol", "layer-shell"] }
"##;

/// Verifies that the config subscription publishes the fully normalized runtime config after hot
/// reload.
#[test]
fn config_subscription_reports_hot_reloaded_runtime_config() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-config-subscription");
    let config_path = runtime_dir.path.join("config-subscription.toml");
    if let Err(error) = fs::write(&config_path, INITIAL_CONFIG) {
        panic!("temporary config should be writable: {error}");
    }

    let mut app = build_app(&config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(240),
    });

    let ipc_socket_path = {
        let world = app.inner().world();
        let Some(server_state) = world.get_resource::<IpcServerState>() else {
            panic!("IPC server state should be available immediately after build");
        };

        match (server_state.listening, &server_state.startup_error) {
            (true, _) => server_state.socket_path.clone(),
            (false, Some(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping config subscription test in restricted environment: {error}");
                return;
            }
            (false, Some(error)) => panic!("IPC startup failed before run: {error}"),
            (false, None) => panic!("IPC startup produced neither socket nor error"),
        }
    };

    let subscription_thread = thread::spawn(move || {
        wait_for_config_change(
            &ipc_socket_path,
            IpcSubscription {
                topic: SubscriptionTopic::Config,
                include_payloads: true,
                events: vec!["config_changed".to_owned()],
            },
        )
    });
    let rewrite_thread = thread::spawn(move || {
        // Give the watcher and subscription stream a brief head start before
        // the rewrite lands.
        thread::sleep(Duration::from_millis(50));
        fs::write(&config_path, RELOADED_CONFIG).map_err(|error| error.to_string())
    });

    if let Err(error) = app.run() {
        panic!("nekoland app should complete the configured frame budget: {error}");
    }

    match rewrite_thread.join() {
        Ok(result) => {
            if let Err(error) = result {
                panic!("config rewrite should succeed: {error}");
            }
        }
        Err(_) => panic!("config rewrite thread should exit cleanly"),
    }

    let event = match subscription_thread.join() {
        Ok(result) => match result {
            Ok(event) => event,
            Err(common::TestControl::Skip(reason)) => {
                eprintln!("skipping config subscription test in restricted environment: {reason}");
                return;
            }
            Err(common::TestControl::Fail(reason)) => {
                panic!("config subscription test failed: {reason}");
            }
        },
        Err(_) => panic!("subscription thread should exit cleanly"),
    };

    assert_eq!(event.topic, SubscriptionTopic::Config);
    assert_eq!(event.event, "config_changed");

    let Some(payload) = event.payload else {
        panic!("config subscription should include a payload");
    };
    let Ok(config) = serde_json::from_value::<ConfigSnapshot>(payload) else {
        panic!("config_changed payload should decode");
    };
    assert_eq!(config.default_layout, "floating");
    assert_eq!(config.command_history_limit, 3);
    assert!(!config.xwayland_enabled);
    assert_eq!(config.configured_keyboard_layout, "de");
    assert_eq!(config.keyboard_layouts.len(), 2);
    assert_eq!(config.keyboard_layouts[1].name, "de");
    assert_eq!(config.keyboard_layouts[1].variant, "nodeadkeys");
    assert_eq!(config.viewport_pan_modifiers, vec!["Ctrl".to_owned(), "Shift".to_owned()]);
    assert_eq!(config.outputs.len(), 1);
    assert_eq!(config.outputs[0].name, "HDMI-A-1");
    assert_eq!(config.outputs[0].mode, "2560x1440@75");
    assert_eq!(config.outputs[0].scale, 2);
    assert!(
        matches!(
            config.keybindings.get("Super+P"),
            Some(actions)
                if actions
                    == &vec![ConfiguredAction::Exec {
                        argv: vec![
                            "wlogout".to_owned(),
                            "--protocol".to_owned(),
                            "layer-shell".to_owned(),
                        ],
                    }]
        ),
        "config_changed should expose the reloaded keybinding map: {:?}",
        config.keybindings
    );
}

/// Waits for the first `config_changed` event on the config subscription stream.
fn wait_for_config_change(
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
                // The subscription may be established before the first reload
                // event is flushed, so short timeouts are expected here.
                if Instant::now() >= deadline {
                    return Err(common::TestControl::Fail(
                        "timed out waiting for config_changed".to_owned(),
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

/// Identifies retryable transient IPC errors.
fn ipc_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Identifies IPC errors that should skip the test in restricted environments.
fn ipc_error_is_skippable(error: &std::io::Error) -> bool {
    // This test also runs under restricted sandboxes, where subscription setup
    // can fail before a usable socket exists. Treat those cases as skips.
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::WouldBlock | ErrorKind::TimedOut
    ) || error.raw_os_error() == Some(1)
}
