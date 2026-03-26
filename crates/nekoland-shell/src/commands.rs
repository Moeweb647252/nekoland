use std::process::Command;

use bevy_ecs::message::{MessageReader, MessageWriter};
use bevy_ecs::prelude::{Res, ResMut, Resource};
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::{CompositorConfig, ConfiguredAction};
use nekoland_core::lifecycle::AppLifecycleState;
use nekoland_ecs::control::{
    OutputControlApi, OutputOps, TilingControlApi, TilingOps, WindowControlApi, WindowOps,
    WorkspaceControlApi, WorkspaceOps,
};
use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
use nekoland_ecs::resources::{
    CommandExecutionRecord, CommandExecutionStatus, CommandHistoryState, CompositorClock,
    ExternalCommandRequest, InputEventRecord, PendingExternalCommandRequests, PendingInputEvents,
    ShortcutRegistry, ShortcutState, ShortcutTrigger, WaylandIngress,
};

/// Tracks whether startup actions have already been applied for this session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct StartupActionState {
    /// Set after startup actions have been evaluated so they only run once per session.
    pub queued: bool,
}

/// Stable shortcut id for quitting the compositor.
pub const SYSTEM_QUIT_SHORTCUT_ID: &str = "system.quit";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ChildCommandEnvironment {
    vars: Vec<(String, String)>,
    removals: Vec<String>,
}

impl ChildCommandEnvironment {
    fn apply_to(&self, command: &mut Command) {
        for key in &self.removals {
            command.env_remove(key);
        }
        for (key, value) in &self.vars {
            command.env(key, value);
        }
    }
}

#[derive(SystemParam)]
/// System parameters required to evaluate config-driven startup actions.
pub struct StartupActionDispatch<'w, 's> {
    config: Res<'w, CompositorConfig>,
    wayland_ingress: Res<'w, WaylandIngress>,
    startup_actions: ResMut<'w, StartupActionState>,
    pending_input_events: ResMut<'w, PendingInputEvents>,
    pending_external_commands: ResMut<'w, PendingExternalCommandRequests>,
    windows: WindowOps<'w, 's>,
    workspaces: WorkspaceOps<'w, 's>,
    outputs: OutputOps<'w, 's>,
    tiling: TilingOps<'w>,
}

struct ActionDispatchContext<'a, 'ops> {
    source: &'a str,
    origin: &'a str,
    pending_input_events: &'a mut PendingInputEvents,
    pending_external_commands: &'a mut PendingExternalCommandRequests,
    windows: &'a mut WindowControlApi<'ops>,
    workspaces: &'a mut WorkspaceControlApi<'ops>,
    outputs: &'a mut OutputControlApi<'ops>,
    tiling: &'a mut TilingControlApi<'ops>,
}

impl<'a, 'ops> ActionDispatchContext<'a, 'ops> {
    fn focused_window(
        &mut self,
        action: &ConfiguredAction,
    ) -> Option<nekoland_ecs::resources::WindowControlHandle<'_>> {
        if let Some(window) = self.windows.focused() {
            return Some(window);
        }

        self.pending_input_events.push(InputEventRecord {
            source: self.source.to_owned(),
            detail: format!("{} -> {} ignored: no focused surface", self.origin, action.describe()),
        });
        None
    }
}

/// Triggers orderly compositor shutdown when `Mod+Shift+Q` is pressed.
pub fn quit_shortcut_system(
    shortcuts: Res<'_, ShortcutState>,
    mut app_lifecycle: ResMut<'_, AppLifecycleState>,
    mut pending_input_events: ResMut<'_, PendingInputEvents>,
) {
    if app_lifecycle.quit_requested || !shortcuts.just_pressed(SYSTEM_QUIT_SHORTCUT_ID) {
        return;
    }

    app_lifecycle.quit_requested = true;
    pending_input_events.push(InputEventRecord {
        source: "keyboard:shortcut".to_owned(),
        detail: "Mod+Shift+Q -> requested compositor quit".to_owned(),
    });
}

/// Registers shell-level shortcuts owned by the command subsystem.
pub fn register_shortcuts(registry: &mut ShortcutRegistry) {
    registry
        .register(nekoland_ecs::resources::ShortcutSpec::new(
            SYSTEM_QUIT_SHORTCUT_ID,
            "system",
            "Request orderly compositor shutdown",
            "Super+Shift+Q",
            ShortcutTrigger::Press,
        ))
        .expect("shell command shortcut ids should be unique");
}

/// Attempts to launch queued external commands and records the result as both ECS messages and
/// human-readable input-log entries.
pub fn external_command_launch_system(
    wayland_ingress: Res<WaylandIngress>,
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

            let child_environment = nested_wayland_env(Some(&wayland_ingress));
            let mut command = Command::new(program);
            command.args(args);
            child_environment.apply_to(&mut command);

            match command.spawn() {
                Ok(child) => {
                    tracing::info!(
                        "Command args={:?}, env={:?}, removed_env={:?} executed",
                        args,
                        child_environment.vars,
                        child_environment.removals
                    );
                    launched_events.write(ExternalCommandLaunched {
                        origin: request.origin.clone(),
                        command: candidate.clone(),
                        pid: child.id(),
                    });
                    pending_input_events.push(InputEventRecord {
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
            pending_input_events.push(InputEventRecord {
                source: "commands".to_owned(),
                detail: format!("{} -> {error}", request.origin),
            });
        }
    }
}

/// Applies startup actions once the nested protocol socket and optional XWayland bridge are ready.
pub fn startup_action_queue_system(startup: StartupActionDispatch<'_, '_>) {
    let StartupActionDispatch {
        config,
        wayland_ingress,
        mut startup_actions,
        mut pending_input_events,
        mut pending_external_commands,
        mut windows,
        mut workspaces,
        mut outputs,
        mut tiling,
    } = startup;

    if startup_actions.queued {
        return;
    }

    if startup_actions_disabled_by_env() {
        startup_actions.queued = true;
        tracing::info!("startup actions disabled by NEKOLAND_DISABLE_STARTUP_COMMANDS");
        return;
    }

    let protocol_server = &wayland_ingress.protocol_server;
    let Some(socket_name) = protocol_server.socket_name.as_deref() else {
        return;
    };

    let xwayland = &wayland_ingress.xwayland_server;
    if xwayland.enabled && !xwayland.ready {
        return;
    }

    startup_actions.queued = true;
    if config.startup_actions.is_empty() {
        return;
    }

    let mut window_controls = windows.api();
    let mut workspace_controls = workspaces.api();
    let mut output_controls = outputs.api();
    let mut tiling_controls = tiling.api();
    let mut dispatch = ActionDispatchContext {
        source: "startup",
        origin: "startup",
        pending_input_events: &mut pending_input_events,
        pending_external_commands: &mut pending_external_commands,
        windows: &mut window_controls,
        workspaces: &mut workspace_controls,
        outputs: &mut output_controls,
        tiling: &mut tiling_controls,
    };
    let mut applied_actions = 0_usize;
    for action in &config.startup_actions {
        if dispatch_configured_action(action, &mut dispatch) {
            applied_actions += 1;
        }
    }

    tracing::info!(
        socket = socket_name,
        runtime_dir = protocol_server.runtime_dir.as_deref().unwrap_or("<unset>"),
        applied_actions,
        "applied startup actions for nested Wayland session"
    );
    pending_input_events.push(InputEventRecord {
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

/// Validates one configured action list before it is compiled into runtime behavior.
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

fn dispatch_configured_action(
    action: &ConfiguredAction,
    dispatch: &mut ActionDispatchContext<'_, '_>,
) -> bool {
    match action {
        ConfiguredAction::CloseFocusedWindow => {
            let Some(mut window) = dispatch.focused_window(action) else {
                return false;
            };
            window.close();
        }
        ConfiguredAction::MoveFocusedWindow { x, y } => {
            let Some(mut window) = dispatch.focused_window(action) else {
                return false;
            };
            window.move_to(*x, *y);
        }
        ConfiguredAction::ResizeFocusedWindow { width, height } => {
            let Some(mut window) = dispatch.focused_window(action) else {
                return false;
            };
            window.resize_to(*width, *height);
        }
        ConfiguredAction::FocusTilingColumn { direction } => {
            dispatch.tiling.controls().focus_column(*direction);
        }
        ConfiguredAction::FocusTilingWindow { direction } => {
            dispatch.tiling.controls().focus_window(*direction);
        }
        ConfiguredAction::MoveTilingColumn { direction } => {
            dispatch.tiling.controls().move_column(*direction);
        }
        ConfiguredAction::MoveTilingWindow { direction } => {
            dispatch.tiling.controls().move_window(*direction);
        }
        ConfiguredAction::ConsumeIntoTilingColumn { direction } => {
            dispatch.tiling.controls().consume_into_column(*direction);
        }
        ConfiguredAction::ExpelFromTilingColumn { direction } => {
            dispatch.tiling.controls().expel_from_column(*direction);
        }
        ConfiguredAction::PanTilingViewport { direction } => {
            dispatch.tiling.controls().pan_viewport(*direction);
        }
        ConfiguredAction::BackgroundFocusedWindow { output } => {
            let Some(mut window) = dispatch.focused_window(action) else {
                return false;
            };
            window.background_on(output.clone());
        }
        ConfiguredAction::ClearFocusedWindowBackground => {
            let Some(mut window) = dispatch.focused_window(action) else {
                return false;
            };
            window.clear_background();
        }
        ConfiguredAction::SwitchWorkspace { workspace } => {
            dispatch.workspaces.switch_or_create(workspace.clone());
        }
        ConfiguredAction::CreateWorkspace { workspace } => {
            dispatch.workspaces.create(workspace.clone());
        }
        ConfiguredAction::DestroyWorkspace { workspace } => {
            dispatch.workspaces.destroy(workspace.clone());
        }
        ConfiguredAction::EnableOutput { output } => {
            dispatch.outputs.named(output.clone()).enable();
        }
        ConfiguredAction::DisableOutput { output } => {
            dispatch.outputs.named(output.clone()).disable();
        }
        ConfiguredAction::ConfigureOutput { output, mode, scale } => {
            dispatch.outputs.named(output.clone()).configure(mode.clone(), *scale);
        }
        ConfiguredAction::PanViewport { delta_x, delta_y } => {
            dispatch.outputs.focused().pan_viewport_by(*delta_x, *delta_y);
        }
        ConfiguredAction::MoveViewport { x, y } => {
            dispatch.outputs.focused().move_viewport_to(*x, *y);
        }
        ConfiguredAction::CenterViewportOnFocusedWindow => {
            let Some(surface_id) = dispatch.windows.focused_surface_id() else {
                dispatch.pending_input_events.push(InputEventRecord {
                    source: dispatch.source.to_owned(),
                    detail: format!(
                        "{} -> {} ignored: no focused surface",
                        dispatch.origin,
                        action.describe()
                    ),
                });
                return false;
            };
            dispatch.outputs.focused().center_viewport_on_window(surface_id);
        }
        ConfiguredAction::Exec { argv } => {
            queue_exec_command(
                format!("{} -> {}", dispatch.origin, action.describe()),
                argv.clone(),
                dispatch.pending_external_commands,
            );
        }
    }

    true
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
fn nested_wayland_env(wayland_ingress: Option<&WaylandIngress>) -> ChildCommandEnvironment {
    let mut env = ChildCommandEnvironment {
        removals: vec![
            "DISPLAY".to_owned(),
            "WAYLAND_DISPLAY".to_owned(),
            "WAYLAND_SOCKET".to_owned(),
            "HYPRLAND_INSTANCE_SIGNATURE".to_owned(),
            "HYPRLAND_CMD".to_owned(),
            "SWAYSOCK".to_owned(),
            "NIRI_SOCKET".to_owned(),
            "I3SOCK".to_owned(),
            "DESKTOP_STARTUP_ID".to_owned(),
            "XDG_ACTIVATION_TOKEN".to_owned(),
        ],
        ..ChildCommandEnvironment::default()
    };

    if let Some(protocol_server) = wayland_ingress.map(|ingress| &ingress.protocol_server) {
        if let Some(socket_name) = protocol_server.socket_name.as_ref() {
            env.vars.push(("WAYLAND_DISPLAY".to_owned(), socket_name.clone()));
        }
        if let Some(runtime_dir) = protocol_server.runtime_dir.as_ref() {
            env.vars.push(("XDG_RUNTIME_DIR".to_owned(), runtime_dir.clone()));
        }
    }

    env.vars.push(("XDG_CURRENT_DESKTOP".to_owned(), "nekoland".to_owned()));
    env.vars.push(("XDG_SESSION_DESKTOP".to_owned(), "nekoland".to_owned()));
    env.vars.push(("DESKTOP_SESSION".to_owned(), "nekoland".to_owned()));
    env.vars.push(("XDG_SESSION_TYPE".to_owned(), "wayland".to_owned()));

    if let Some(display_name) = wayland_ingress
        .map(|ingress| &ingress.xwayland_server)
        .and_then(|state| state.ready.then_some(state.display_name.as_deref()).flatten())
    {
        env.vars.push(("DISPLAY".to_owned(), display_name.to_owned()));
    }

    env
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_config::resources::{CompositorConfig, ConfiguredAction};
    use nekoland_core::lifecycle::AppLifecycleState;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
    use nekoland_ecs::resources::PendingInputEvents;
    use nekoland_ecs::resources::{
        CommandHistoryState, CompositorClock, KeyboardFocusState, PendingExternalCommandRequests,
        PendingOutputControls, PendingWindowControls, PendingWorkspaceControls, ProtocolServerState,
        ShortcutState, WaylandIngress, XWaylandServerState,
    };

    use super::{StartupActionState, quit_shortcut_system, startup_action_queue_system};

    #[test]
    fn mod_shift_q_requests_quit_once() {
        let mut app = NekolandApp::new("quit-shortcut-test");
        app.insert_resource(AppLifecycleState::default())
            .insert_resource(ShortcutState::default())
            .insert_resource(PendingInputEvents::default())
            .inner_mut()
            .add_systems(LayoutSchedule, quit_shortcut_system);

        app.inner_mut()
            .world_mut()
            .resource_mut::<ShortcutState>()
            .set(super::SYSTEM_QUIT_SHORTCUT_ID, true, true, false);

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert!(world.resource::<AppLifecycleState>().quit_requested);
        assert_eq!(world.resource::<PendingInputEvents>().iter().count(), 1);
    }

    #[test]
    fn plain_q_does_not_request_quit() {
        let mut app = NekolandApp::new("quit-shortcut-negative-test");
        app.insert_resource(AppLifecycleState::default())
            .insert_resource(ShortcutState::default())
            .insert_resource(PendingInputEvents::default())
            .inner_mut()
            .add_systems(LayoutSchedule, quit_shortcut_system);
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        assert!(!world.resource::<AppLifecycleState>().quit_requested);
        assert_eq!(world.resource::<PendingInputEvents>().iter().count(), 0);
    }

    #[test]
    fn nested_wayland_env_scrubs_host_display_leaks_without_xwayland() {
        let env = super::nested_wayland_env(Some(&WaylandIngress {
            protocol_server: ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            },
            xwayland_server: XWaylandServerState {
                enabled: true,
                ready: false,
                display_name: Some(":77".to_owned()),
                ..XWaylandServerState::default()
            },
            ..WaylandIngress::default()
        }));

        let vars = env.vars.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(vars.get("WAYLAND_DISPLAY"), Some(&"wayland-77".to_owned()));
        assert_eq!(vars.get("XDG_RUNTIME_DIR"), Some(&"/tmp/nekoland-runtime".to_owned()));
        assert_eq!(vars.get("XDG_CURRENT_DESKTOP"), Some(&"nekoland".to_owned()));
        assert_eq!(vars.get("XDG_SESSION_DESKTOP"), Some(&"nekoland".to_owned()));
        assert_eq!(vars.get("DESKTOP_SESSION"), Some(&"nekoland".to_owned()));
        assert_eq!(vars.get("XDG_SESSION_TYPE"), Some(&"wayland".to_owned()));
        assert!(
            !vars.contains_key("DISPLAY"),
            "host DISPLAY should be removed until nested XWayland is ready"
        );
        assert_eq!(
            env.removals,
            vec![
                "DISPLAY".to_owned(),
                "WAYLAND_DISPLAY".to_owned(),
                "WAYLAND_SOCKET".to_owned(),
                "HYPRLAND_INSTANCE_SIGNATURE".to_owned(),
                "HYPRLAND_CMD".to_owned(),
                "SWAYSOCK".to_owned(),
                "NIRI_SOCKET".to_owned(),
                "I3SOCK".to_owned(),
                "DESKTOP_STARTUP_ID".to_owned(),
                "XDG_ACTIVATION_TOKEN".to_owned(),
            ]
        );
    }

    #[test]
    fn nested_wayland_env_sets_nested_display_when_xwayland_ready() {
        let env = super::nested_wayland_env(Some(&WaylandIngress {
            protocol_server: ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            },
            xwayland_server: XWaylandServerState {
                enabled: true,
                ready: true,
                display_name: Some(":77".to_owned()),
                ..XWaylandServerState::default()
            },
            ..WaylandIngress::default()
        }));

        let vars = env.vars.iter().cloned().collect::<BTreeMap<_, _>>();
        assert_eq!(vars.get("DISPLAY"), Some(&":77".to_owned()));
        assert_eq!(vars.get("WAYLAND_DISPLAY"), Some(&"wayland-77".to_owned()));
        assert_eq!(vars.get("XDG_SESSION_DESKTOP"), Some(&"nekoland".to_owned()));
        assert_eq!(vars.get("DESKTOP_SESSION"), Some(&"nekoland".to_owned()));
    }

    #[test]
    fn startup_actions_wait_for_xwayland_ready_when_enabled() {
        let config = CompositorConfig {
            startup_actions: vec![ConfiguredAction::Exec { argv: vec!["true".to_owned()] }],
            ..CompositorConfig::default()
        };
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
            .insert_resource(WaylandIngress {
                protocol_server: ProtocolServerState {
                    socket_name: Some("wayland-77".to_owned()),
                    runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                    ..ProtocolServerState::default()
                },
                xwayland_server: XWaylandServerState {
                    enabled: true,
                    ready: false,
                    ..XWaylandServerState::default()
                },
                ..WaylandIngress::default()
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
        let Some(startup_state) = world.get_resource::<StartupActionState>() else {
            panic!("startup action state should exist");
        };
        assert!(!startup_state.queued, "should wait for xwayland ready");

        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().xwayland_server.ready = true;
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let Some(startup_state) = world.get_resource::<StartupActionState>() else {
            panic!("startup action state should exist");
        };
        let Some(history) = world.get_resource::<CommandHistoryState>() else {
            panic!("command history state should exist");
        };
        assert!(startup_state.queued, "should be queued after xwayland ready");
        assert_eq!(history.items.len(), 1, "should have executed after xwayland ready");
    }

    #[test]
    fn startup_actions_run_immediately_when_xwayland_disabled() {
        let config = CompositorConfig {
            startup_actions: vec![ConfiguredAction::Exec { argv: vec!["true".to_owned()] }],
            ..CompositorConfig::default()
        };
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
            .insert_resource(WaylandIngress {
                protocol_server: ProtocolServerState {
                    socket_name: Some("wayland-77".to_owned()),
                    runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                    ..ProtocolServerState::default()
                },
                xwayland_server: XWaylandServerState {
                    enabled: false,
                    ready: false,
                    ..XWaylandServerState::default()
                },
                ..WaylandIngress::default()
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
        let Some(startup_state) = world.get_resource::<StartupActionState>() else {
            panic!("startup action state should exist");
        };
        let Some(history) = world.get_resource::<CommandHistoryState>() else {
            panic!("command history state should exist");
        };
        assert!(startup_state.queued, "should run when xwayland disabled");
        assert_eq!(history.items.len(), 1, "should have executed");
    }

    #[test]
    fn startup_actions_run_immediately_when_xwayland_not_present() {
        let config = CompositorConfig {
            startup_actions: vec![ConfiguredAction::Exec { argv: vec!["true".to_owned()] }],
            ..CompositorConfig::default()
        };
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
            .insert_resource(WaylandIngress {
                protocol_server: ProtocolServerState {
                    socket_name: Some("wayland-77".to_owned()),
                    runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                    ..ProtocolServerState::default()
                },
                ..WaylandIngress::default()
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
        let Some(startup_state) = world.get_resource::<StartupActionState>() else {
            panic!("startup action state should exist");
        };
        let Some(history) = world.get_resource::<CommandHistoryState>() else {
            panic!("command history state should exist");
        };
        assert!(startup_state.queued, "should run when no xwayland resource");
        assert_eq!(history.items.len(), 1, "should have executed");
    }
}
