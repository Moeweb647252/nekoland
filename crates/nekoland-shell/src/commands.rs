use std::process::Command;

use bevy_ecs::message::{MessageReader, MessageWriter};
use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::control::{
    OutputControlApi, OutputOps, WindowControlApi, WindowOps, WorkspaceControlApi, WorkspaceOps,
};
use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
use nekoland_ecs::resources::{
    CommandExecutionRecord, CommandExecutionStatus, CommandHistoryState, CompositorClock,
    CompositorConfig, ConfiguredAction, ExternalCommandRequest, PendingExternalCommandRequests,
    PendingInputEvents, describe_action_sequence,
};
use nekoland_protocol::{ProtocolServerState, XWaylandServerState};

/// Tracks whether startup actions have already been applied for this session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct StartupActionState {
    pub queued: bool,
}

/// Attempts to launch queued external commands and records the result as both ECS messages and
/// human-readable input-log entries.
pub fn external_command_launch_system(
    protocol_server: Option<Res<ProtocolServerState>>,
    xwayland_server: Option<Res<XWaylandServerState>>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut launched_events: MessageWriter<ExternalCommandLaunched>,
    mut failed_events: MessageWriter<ExternalCommandFailed>,
) {
    for request in pending_external_commands.drain() {
        let mut last_error = None;
        let mut launched = false;

        for candidate in &request.candidates {
            let Some((program, args)) = candidate.split_first() else {
                continue;
            };

            let nested_wayland_env =
                nested_wayland_env(protocol_server.as_deref(), xwayland_server.as_deref());
            let mut command = Command::new(program);
            command.args(args);
            for (key, value) in &nested_wayland_env {
                command.env(key, value);
            }

            match command.spawn() {
                Ok(child) => {
                    tracing::info!(
                        "Command args={:?}, env={:?} executed",
                        args,
                        nested_wayland_env
                    );
                    launched_events.write(ExternalCommandLaunched {
                        origin: request.origin.clone(),
                        command: candidate.clone(),
                        pid: child.id(),
                    });
                    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                        source: "commands".to_owned(),
                        detail: format!(
                            "{} -> launched `{}` (pid {})",
                            request.origin,
                            candidate.join(" "),
                            child.id()
                        ),
                    });
                    launched = true;
                    break;
                }
                Err(error) => {
                    last_error = Some(format!("{}: {error}", candidate.join(" ")));
                }
            }
        }

        if !launched {
            let error = last_error.unwrap_or_else(|| {
                "ignored because no executable candidates were available".to_owned()
            });
            failed_events.write(ExternalCommandFailed {
                origin: request.origin.clone(),
                candidates: request.candidates.clone(),
                error: error.clone(),
            });
            pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                source: "commands".to_owned(),
                detail: format!("{} -> {error}", request.origin),
            });
        }
    }
}

/// Applies startup actions once the nested protocol socket and optional XWayland bridge are ready.
pub fn startup_action_queue_system(
    config: Res<CompositorConfig>,
    protocol_server: Option<Res<ProtocolServerState>>,
    xwayland_server: Option<Res<XWaylandServerState>>,
    mut startup_actions: ResMut<StartupActionState>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
    mut windows: WindowOps,
    mut workspaces: WorkspaceOps,
    mut outputs: OutputOps,
) {
    if startup_actions.queued {
        return;
    }

    if startup_actions_disabled_by_env() {
        startup_actions.queued = true;
        tracing::info!("startup actions disabled by NEKOLAND_DISABLE_STARTUP_COMMANDS");
        return;
    }

    let Some(protocol_server) = protocol_server else {
        return;
    };
    let Some(socket_name) = protocol_server.socket_name.as_deref() else {
        return;
    };

    if let Some(xwayland) = xwayland_server.as_deref() {
        if xwayland.enabled && !xwayland.ready {
            return;
        }
    }

    startup_actions.queued = true;
    if config.startup_actions.is_empty() {
        return;
    }

    let mut window_controls = windows.api();
    let mut workspace_controls = workspaces.api();
    let mut output_controls = outputs.api();
    let mut applied_actions = 0_usize;
    for action in &config.startup_actions {
        if dispatch_configured_action(
            "startup",
            "startup",
            action,
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut window_controls,
            &mut workspace_controls,
            &mut output_controls,
        ) {
            applied_actions += 1;
        }
    }

    tracing::info!(
        socket = socket_name,
        runtime_dir = protocol_server.runtime_dir.as_deref().unwrap_or("<unset>"),
        applied_actions,
        "applied startup actions for nested Wayland session"
    );
    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: "startup".to_owned(),
        detail: format!("applied {applied_actions} startup action(s) for {socket_name}"),
    });
}

/// Folds command launch/failure messages into the bounded command history resource.
pub fn command_history_system(
    config: Res<CompositorConfig>,
    clock: Res<CompositorClock>,
    mut launched: MessageReader<ExternalCommandLaunched>,
    mut failed: MessageReader<ExternalCommandFailed>,
    mut history: ResMut<CommandHistoryState>,
) {
    if history.limit != config.command_history_limit {
        history.limit = config.command_history_limit;
        if history.limit == 0 {
            history.items.clear();
        } else if history.items.len() > history.limit {
            let overflow = history.items.len() - history.limit;
            history.items.drain(..overflow);
        }
    }

    for event in launched.read() {
        history.push(CommandExecutionRecord {
            frame: clock.frame,
            uptime_millis: clock.uptime_millis,
            origin: event.origin.clone(),
            command: Some(event.command.clone()),
            candidates: vec![event.command.clone()],
            status: Some(CommandExecutionStatus::Launched { pid: event.pid }),
        });
    }

    for event in failed.read() {
        history.push(CommandExecutionRecord {
            frame: clock.frame,
            uptime_millis: clock.uptime_millis,
            origin: event.origin.clone(),
            command: None,
            candidates: event.candidates.clone(),
            status: Some(CommandExecutionStatus::Failed { error: event.error.clone() }),
        });
    }
}

/// Convenience helper used by keybindings and other shell systems to enqueue one exact argv.
pub fn queue_exec_command(
    origin: String,
    argv: Vec<String>,
    pending_external_commands: &mut PendingExternalCommandRequests,
) {
    pending_external_commands.push(ExternalCommandRequest { origin, candidates: vec![argv] });
}

pub fn validate_action_sequence(actions: &[ConfiguredAction]) -> Result<(), String> {
    if actions.is_empty() {
        return Err("action sequence must contain at least one action".to_owned());
    }
    for action in actions {
        validate_action(action)?;
    }
    Ok(())
}

fn validate_action(action: &ConfiguredAction) -> Result<(), String> {
    match action {
        ConfiguredAction::Exec { argv } => {
            let Some(program) = argv.first() else {
                return Err("command action must include at least one argv element".to_owned());
            };
            if program.trim().is_empty() {
                return Err("command action must not start with an empty program".to_owned());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub fn dispatch_action_sequence(
    source: &str,
    origin: &str,
    actions: &[ConfiguredAction],
    pending_input_events: &mut PendingInputEvents,
    pending_external_commands: &mut PendingExternalCommandRequests,
    windows: &mut WindowControlApi<'_>,
    workspaces: &mut WorkspaceControlApi<'_>,
    outputs: &mut OutputControlApi<'_>,
) -> bool {
    for action in actions {
        if !dispatch_configured_action(
            source,
            origin,
            action,
            pending_input_events,
            pending_external_commands,
            windows,
            workspaces,
            outputs,
        ) {
            return false;
        }
    }

    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: source.to_owned(),
        detail: format!("{origin} -> {}", describe_action_sequence(actions)),
    });
    true
}

pub fn dispatch_configured_action(
    source: &str,
    origin: &str,
    action: &ConfiguredAction,
    pending_input_events: &mut PendingInputEvents,
    pending_external_commands: &mut PendingExternalCommandRequests,
    windows: &mut WindowControlApi<'_>,
    workspaces: &mut WorkspaceControlApi<'_>,
    outputs: &mut OutputControlApi<'_>,
) -> bool {
    match action {
        ConfiguredAction::CloseFocusedWindow => {
            let Some(mut window) =
                focused_window(source, origin, action, pending_input_events, windows)
            else {
                return false;
            };
            window.close();
        }
        ConfiguredAction::MoveFocusedWindow { x, y } => {
            let Some(mut window) =
                focused_window(source, origin, action, pending_input_events, windows)
            else {
                return false;
            };
            window.move_to(*x, *y);
        }
        ConfiguredAction::ResizeFocusedWindow { width, height } => {
            let Some(mut window) =
                focused_window(source, origin, action, pending_input_events, windows)
            else {
                return false;
            };
            window.resize_to(*width, *height);
        }
        ConfiguredAction::SplitFocusedWindow { axis } => {
            let Some(mut window) =
                focused_window(source, origin, action, pending_input_events, windows)
            else {
                return false;
            };
            window.split(*axis);
        }
        ConfiguredAction::BackgroundFocusedWindow { output } => {
            let Some(mut window) =
                focused_window(source, origin, action, pending_input_events, windows)
            else {
                return false;
            };
            window.background_on(output.clone());
        }
        ConfiguredAction::ClearFocusedWindowBackground => {
            let Some(mut window) =
                focused_window(source, origin, action, pending_input_events, windows)
            else {
                return false;
            };
            window.clear_background();
        }
        ConfiguredAction::SwitchWorkspace { workspace } => {
            workspaces.switch_or_create(workspace.clone());
        }
        ConfiguredAction::CreateWorkspace { workspace } => {
            workspaces.create(workspace.clone());
        }
        ConfiguredAction::DestroyWorkspace { workspace } => {
            workspaces.destroy(workspace.clone());
        }
        ConfiguredAction::EnableOutput { output } => {
            outputs.named(output.clone()).enable();
        }
        ConfiguredAction::DisableOutput { output } => {
            outputs.named(output.clone()).disable();
        }
        ConfiguredAction::ConfigureOutput { output, mode, scale } => {
            outputs.named(output.clone()).configure(mode.clone(), *scale);
        }
        ConfiguredAction::PanViewport { delta_x, delta_y } => {
            outputs.focused().pan_viewport_by(*delta_x, *delta_y);
        }
        ConfiguredAction::MoveViewport { x, y } => {
            outputs.focused().move_viewport_to(*x, *y);
        }
        ConfiguredAction::CenterViewportOnFocusedWindow => {
            let Some(surface_id) = windows.focused_surface_id() else {
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: source.to_owned(),
                    detail: format!(
                        "{origin} -> {} ignored: no focused surface",
                        action.describe()
                    ),
                });
                return false;
            };
            outputs.focused().center_viewport_on_window(surface_id);
        }
        ConfiguredAction::Exec { argv } => {
            queue_exec_command(
                format!("{origin} -> {}", action.describe()),
                argv.clone(),
                pending_external_commands,
            );
        }
    }

    true
}

fn focused_window<'a>(
    source: &str,
    origin: &str,
    action: &ConfiguredAction,
    pending_input_events: &mut PendingInputEvents,
    windows: &'a mut WindowControlApi<'_>,
) -> Option<nekoland_ecs::resources::WindowControlHandle<'a>> {
    if let Some(window) = windows.focused() {
        return Some(window);
    }

    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: source.to_owned(),
        detail: format!("{origin} -> {} ignored: no focused surface", action.describe()),
    });
    None
}

/// Allows tests or nested sessions to disable startup actions entirely via environment variable.
fn startup_actions_disabled_by_env() -> bool {
    std::env::var_os("NEKOLAND_DISABLE_STARTUP_COMMANDS").is_some_and(|value| {
        let value = value.to_string_lossy();
        !value.is_empty()
            && !value.eq_ignore_ascii_case("0")
            && !value.eq_ignore_ascii_case("false")
    })
}

/// Builds the environment variables needed for child processes to connect to the nested Wayland
/// and optional XWayland session created by the compositor.
fn nested_wayland_env(
    protocol_server: Option<&ProtocolServerState>,
    xwayland_server: Option<&XWaylandServerState>,
) -> Vec<(String, String)> {
    let mut env = Vec::new();
    let Some(protocol_server) = protocol_server else {
        return env;
    };

    if let Some(socket_name) = protocol_server.socket_name.as_ref() {
        env.push(("WAYLAND_DISPLAY".to_owned(), socket_name.clone()));
    }
    if let Some(runtime_dir) = protocol_server.runtime_dir.as_ref() {
        env.push(("XDG_RUNTIME_DIR".to_owned(), runtime_dir.clone()));
    }
    if let Some(display_name) = xwayland_server
        .and_then(|state| state.ready.then_some(state.display_name.as_deref()).flatten())
    {
        env.push(("DISPLAY".to_owned(), display_name.to_owned()));
    }

    env
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
    use nekoland_ecs::resources::{
        CommandHistoryState, CompositorClock, CompositorConfig, ConfiguredAction,
        KeyboardFocusState, PendingExternalCommandRequests, PendingInputEvents,
        PendingOutputControls, PendingWindowControls, PendingWorkspaceControls,
    };
    use nekoland_protocol::{ProtocolServerState, XWaylandServerState};

    use super::{StartupActionState, startup_action_queue_system};

    #[test]
    fn startup_actions_wait_for_xwayland_ready_when_enabled() {
        let mut config = CompositorConfig::default();
        config.startup_actions = vec![ConfiguredAction::Exec { argv: vec!["true".to_owned()] }];
        let mut app = NekolandApp::new("startup-xwayland-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(config)
            .insert_resource(StartupActionState::default())
            .insert_resource(CommandHistoryState::default())
            .insert_resource(PendingExternalCommandRequests::default())
            .insert_resource(PendingWindowControls::default())
            .insert_resource(PendingWorkspaceControls::default())
            .insert_resource(PendingOutputControls::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(PendingInputEvents::default())
            .insert_resource(ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            })
            .insert_resource(XWaylandServerState {
                enabled: true,
                ready: false,
                ..XWaylandServerState::default()
            })
            .inner_mut()
            .add_message::<ExternalCommandLaunched>()
            .add_message::<ExternalCommandFailed>()
            .add_systems(
                LayoutSchedule,
                (
                    startup_action_queue_system,
                    super::external_command_launch_system,
                    super::command_history_system,
                )
                    .chain(),
            );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let startup_state = world.get_resource::<StartupActionState>().unwrap();
        assert!(!startup_state.queued, "should wait for xwayland ready");

        app.inner_mut().world_mut().resource_mut::<XWaylandServerState>().ready = true;
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let startup_state = world.get_resource::<StartupActionState>().unwrap();
        let history = world.get_resource::<CommandHistoryState>().unwrap();
        assert!(startup_state.queued, "should be queued after xwayland ready");
        assert_eq!(history.items.len(), 1, "should have executed after xwayland ready");
    }

    #[test]
    fn startup_actions_run_immediately_when_xwayland_disabled() {
        let mut config = CompositorConfig::default();
        config.startup_actions = vec![ConfiguredAction::Exec { argv: vec!["true".to_owned()] }];
        let mut app = NekolandApp::new("startup-xwayland-disabled-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(config)
            .insert_resource(StartupActionState::default())
            .insert_resource(CommandHistoryState::default())
            .insert_resource(PendingExternalCommandRequests::default())
            .insert_resource(PendingWindowControls::default())
            .insert_resource(PendingWorkspaceControls::default())
            .insert_resource(PendingOutputControls::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(PendingInputEvents::default())
            .insert_resource(ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            })
            .insert_resource(XWaylandServerState {
                enabled: false,
                ready: false,
                ..XWaylandServerState::default()
            })
            .inner_mut()
            .add_message::<ExternalCommandLaunched>()
            .add_message::<ExternalCommandFailed>()
            .add_systems(
                LayoutSchedule,
                (
                    startup_action_queue_system,
                    super::external_command_launch_system,
                    super::command_history_system,
                )
                    .chain(),
            );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let startup_state = world.get_resource::<StartupActionState>().unwrap();
        let history = world.get_resource::<CommandHistoryState>().unwrap();
        assert!(startup_state.queued, "should run when xwayland disabled");
        assert_eq!(history.items.len(), 1, "should have executed");
    }

    #[test]
    fn startup_actions_run_immediately_when_xwayland_not_present() {
        let mut config = CompositorConfig::default();
        config.startup_actions = vec![ConfiguredAction::Exec { argv: vec!["true".to_owned()] }];
        let mut app = NekolandApp::new("startup-no-xwayland-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(config)
            .insert_resource(StartupActionState::default())
            .insert_resource(CommandHistoryState::default())
            .insert_resource(PendingExternalCommandRequests::default())
            .insert_resource(PendingWindowControls::default())
            .insert_resource(PendingWorkspaceControls::default())
            .insert_resource(PendingOutputControls::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(PendingInputEvents::default())
            .insert_resource(ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            })
            .inner_mut()
            .add_message::<ExternalCommandLaunched>()
            .add_message::<ExternalCommandFailed>()
            .add_systems(
                LayoutSchedule,
                (
                    startup_action_queue_system,
                    super::external_command_launch_system,
                    super::command_history_system,
                )
                    .chain(),
            );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let startup_state = world.get_resource::<StartupActionState>().unwrap();
        let history = world.get_resource::<CommandHistoryState>().unwrap();
        assert!(startup_state.queued, "should run when no xwayland resource");
        assert_eq!(history.items.len(), 1, "should have executed");
    }
}
