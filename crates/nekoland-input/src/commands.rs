use std::process::Command;

use bevy_ecs::message::{MessageReader, MessageWriter};
use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
use nekoland_ecs::resources::{
    CommandExecutionRecord, CommandExecutionStatus, CommandHistoryState, CompositorClock,
    CompositorConfig, ExternalCommandConfig, ExternalCommandRequest,
    PendingExternalCommandRequests, PendingInputEvents,
};
use nekoland_protocol::{ProtocolServerState, XWaylandServerState};

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct StartupCommandState {
    pub queued: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalCommandKind {
    Terminal,
    Launcher,
    PowerMenu,
}

pub fn external_command_launch_system(
    protocol_server: Option<Res<ProtocolServerState>>,
    xwayland_server: Option<Res<XWaylandServerState>>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut launched_events: MessageWriter<ExternalCommandLaunched>,
    mut failed_events: MessageWriter<ExternalCommandFailed>,
) {
    for request in pending_external_commands.items.drain(..) {
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
                    launched_events.write(ExternalCommandLaunched {
                        origin: request.origin.clone(),
                        command: candidate.clone(),
                        pid: child.id(),
                    });
                    pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
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
            pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
                source: "commands".to_owned(),
                detail: format!("{} -> {error}", request.origin),
            });
        }
    }
}

pub fn startup_command_queue_system(
    config: Res<CompositorConfig>,
    protocol_server: Option<Res<ProtocolServerState>>,
    mut startup_commands: ResMut<StartupCommandState>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
) {
    if startup_commands.queued {
        return;
    }

    if startup_commands_disabled_by_env() {
        startup_commands.queued = true;
        tracing::info!("startup commands disabled by NEKOLAND_DISABLE_STARTUP_COMMANDS");
        return;
    }

    let Some(protocol_server) = protocol_server else {
        return;
    };
    let Some(socket_name) = protocol_server.socket_name.as_deref() else {
        return;
    };

    startup_commands.queued = true;
    if config.startup_commands.is_empty() {
        return;
    }

    let runtime_dir = protocol_server.runtime_dir.clone();
    let mut queued_commands = 0_usize;
    for command in &config.startup_commands {
        let argv = split_command_line(command);
        if argv.is_empty() {
            pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
                source: "startup".to_owned(),
                detail: "ignored empty startup command".to_owned(),
            });
            continue;
        }

        pending_external_commands.items.push(ExternalCommandRequest {
            origin: format!("startup -> {command}"),
            candidates: vec![argv],
        });
        queued_commands += 1;
    }

    tracing::info!(
        socket = socket_name,
        runtime_dir = runtime_dir.as_deref().unwrap_or("<unset>"),
        queued_commands,
        "queued startup commands for nested Wayland session"
    );
    pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
        source: "startup".to_owned(),
        detail: format!("queued {queued_commands} startup command(s) for {socket_name}"),
    });
}

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

pub(crate) fn queue_external_command(
    origin: String,
    kind: ExternalCommandKind,
    command_config: &ExternalCommandConfig,
    pending_external_commands: &mut PendingExternalCommandRequests,
) {
    pending_external_commands.items.push(ExternalCommandRequest {
        origin,
        candidates: command_candidates(kind, command_config),
    });
}

pub(crate) fn queue_exec_command(
    origin: String,
    argv: Vec<String>,
    pending_external_commands: &mut PendingExternalCommandRequests,
) {
    pending_external_commands.items.push(ExternalCommandRequest { origin, candidates: vec![argv] });
}

pub(crate) fn command_candidates(
    kind: ExternalCommandKind,
    command_config: &ExternalCommandConfig,
) -> Vec<Vec<String>> {
    let mut candidates = Vec::new();

    if let Some(configured) = command_from_config(kind, command_config) {
        candidates.push(configured);
    }

    for fallback in fallback_command_candidates(kind) {
        if !candidates.contains(&fallback) {
            candidates.push(fallback);
        }
    }

    candidates
}

pub(crate) fn split_command_line(command: &str) -> Vec<String> {
    parse_command_line(command)
        .filter(|argv| !argv.is_empty())
        .unwrap_or_else(|| command.split_whitespace().map(str::to_owned).collect())
}

fn fallback_command_candidates(kind: ExternalCommandKind) -> Vec<Vec<String>> {
    match kind {
        ExternalCommandKind::Terminal => ["foot", "wezterm", "alacritty", "kitty", "xterm"]
            .into_iter()
            .map(|program| vec![program.to_owned()])
            .collect(),
        ExternalCommandKind::Launcher => vec![
            vec!["fuzzel".to_owned()],
            vec!["wofi".to_owned(), "--show".to_owned(), "drun".to_owned()],
            vec!["rofi".to_owned(), "-show".to_owned(), "drun".to_owned()],
            vec!["bemenu-run".to_owned()],
        ],
        ExternalCommandKind::PowerMenu => {
            vec![vec!["wlogout".to_owned()], vec!["nwg-bar".to_owned()]]
        }
    }
}

fn command_from_config(
    kind: ExternalCommandKind,
    command_config: &ExternalCommandConfig,
) -> Option<Vec<String>> {
    let command = match kind {
        ExternalCommandKind::Terminal => command_config.terminal.as_deref(),
        ExternalCommandKind::Launcher => command_config.launcher.as_deref(),
        ExternalCommandKind::PowerMenu => command_config.power_menu.as_deref(),
    }?;
    let argv = split_command_line(command);
    (!argv.is_empty()).then_some(argv)
}

fn startup_commands_disabled_by_env() -> bool {
    std::env::var_os("NEKOLAND_DISABLE_STARTUP_COMMANDS").is_some_and(|value| {
        let value = value.to_string_lossy();
        !value.is_empty()
            && !value.eq_ignore_ascii_case("0")
            && !value.eq_ignore_ascii_case("false")
    })
}

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

fn parse_command_line(command: &str) -> Option<Vec<String>> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = command.chars();

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            Some('"') => match ch {
                '"' => quote = None,
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    } else {
                        current.push('\\');
                    }
                }
                _ => current.push(ch),
            },
            Some(_) => unreachable!(),
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    } else {
                        current.push('\\');
                    }
                }
                whitespace if whitespace.is_whitespace() => {
                    if !current.is_empty() {
                        argv.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
        }
    }

    if quote.is_some() {
        return None;
    }

    if !current.is_empty() {
        argv.push(current);
    }

    Some(argv)
}
