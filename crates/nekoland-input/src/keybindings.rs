use std::collections::BTreeMap;

use crate::commands::{self, ExternalCommandKind};
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::events::KeyPress;
use nekoland_ecs::resources::{
    CompositorConfig, ExternalCommandConfig, KeyboardFocusState, ModifierState, OutputServerAction,
    OutputServerRequest, PendingExternalCommandRequests, PendingInputEvents,
    PendingOutputServerRequests, PendingWindowServerRequests, PendingWorkspaceServerRequests,
    WindowServerAction, WindowServerRequest, WorkspaceServerAction, WorkspaceServerRequest,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct KeybindingEngine {
    pub bindings_loaded: usize,
    loaded_bindings: BTreeMap<String, String>,
    compiled_bindings: Vec<CompiledKeybinding>,
}

pub fn keybinding_dispatch_system(
    config: Res<CompositorConfig>,
    modifiers: Res<ModifierState>,
    keyboard_focus: Res<KeyboardFocusState>,
    mut engine: ResMut<KeybindingEngine>,
    mut key_events: MessageReader<KeyPress>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    mut pending_workspace_requests: ResMut<PendingWorkspaceServerRequests>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
) {
    if engine.loaded_bindings != config.keybindings {
        engine.reload_bindings(&config.keybindings, &mut pending_input_events);
    }

    let mut observed = 0_usize;
    let modifiers = modifiers.into_inner();
    let keyboard_focus = keyboard_focus.into_inner();
    for event in key_events.read() {
        if !event.pressed {
            continue;
        }

        observed += 1;
        let Some(binding) = engine.match_binding(event.keycode, modifiers) else {
            continue;
        };

        dispatch_keybinding_action(
            binding,
            &config.commands,
            keyboard_focus,
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut pending_window_requests,
            &mut pending_workspace_requests,
            &mut pending_output_requests,
        );
    }

    tracing::trace!(observed, bindings_loaded = engine.bindings_loaded, "keybinding dispatch tick");
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledKeybinding {
    chord: KeyChord,
    action: KeybindingAction,
    binding: String,
    command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyChord {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
    keycode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeybindingAction {
    CloseFocusedWindow,
    MoveFocusedWindow { x: i32, y: i32 },
    ResizeFocusedWindow { width: u32, height: u32 },
    SwitchWorkspace(String),
    CreateWorkspace(String),
    DestroyWorkspace(String),
    EnableOutput(String),
    DisableOutput(String),
    ConfigureOutput { output: String, mode: String, scale: Option<u32> },
    LaunchTerminal,
    LaunchApplicationLauncher,
    ShowPowerMenu,
    Exec(Vec<String>),
}

impl KeybindingEngine {
    fn reload_bindings(
        &mut self,
        bindings: &BTreeMap<String, String>,
        pending_input_events: &mut PendingInputEvents,
    ) {
        self.loaded_bindings = bindings.clone();
        self.compiled_bindings.clear();

        for (binding, command) in bindings {
            match compile_keybinding(binding, command) {
                Ok(compiled) => self.compiled_bindings.push(compiled),
                Err(error) => {
                    tracing::warn!(binding, command, error, "ignoring invalid keybinding");
                    pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
                        source: "keybinding".to_owned(),
                        detail: format!("{binding} -> {command} ignored: {error}"),
                    });
                }
            }
        }

        self.bindings_loaded = self.compiled_bindings.len();
    }

    fn match_binding(
        &self,
        keycode: u32,
        modifiers: &ModifierState,
    ) -> Option<&CompiledKeybinding> {
        self.compiled_bindings.iter().find(|binding| binding.chord.matches(keycode, modifiers))
    }
}

impl KeyChord {
    fn matches(&self, keycode: u32, modifiers: &ModifierState) -> bool {
        self.keycode == keycode
            && self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
            && self.logo == modifiers.logo
    }
}

fn compile_keybinding(binding: &str, command: &str) -> Result<CompiledKeybinding, String> {
    Ok(CompiledKeybinding {
        chord: parse_key_chord(binding)?,
        action: parse_keybinding_action(command)?,
        binding: binding.to_owned(),
        command: command.to_owned(),
    })
}

fn parse_key_chord(binding: &str) -> Result<KeyChord, String> {
    let mut chord = KeyChord { ctrl: false, alt: false, shift: false, logo: false, keycode: 0 };
    let mut keycode = None;

    for token in binding.split('+').map(str::trim).filter(|token| !token.is_empty()) {
        match normalize_modifier_name(token) {
            Some("ctrl") => chord.ctrl = true,
            Some("alt") => chord.alt = true,
            Some("shift") => chord.shift = true,
            Some("logo") => chord.logo = true,
            Some(_) => unreachable!(),
            None => {
                if keycode.is_some() {
                    return Err("binding must contain exactly one non-modifier key".to_owned());
                }
                keycode = Some(parse_keycode(token)?);
            }
        }
    }

    chord.keycode = keycode.ok_or_else(|| "binding is missing a non-modifier key".to_owned())?;
    Ok(chord)
}

fn parse_keybinding_action(command: &str) -> Result<KeybindingAction, String> {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        ["close-window"] | ["window", "close"] => Ok(KeybindingAction::CloseFocusedWindow),
        ["window", "move", x, y] => Ok(KeybindingAction::MoveFocusedWindow {
            x: parse_i32("window move x", x)?,
            y: parse_i32("window move y", y)?,
        }),
        ["window", "resize", width, height] => Ok(KeybindingAction::ResizeFocusedWindow {
            width: parse_u32("window resize width", width)?,
            height: parse_u32("window resize height", height)?,
        }),
        ["workspace", workspace] => Ok(KeybindingAction::SwitchWorkspace((*workspace).to_owned())),
        ["workspace", "switch", workspace] | ["workspace-switch", workspace] => {
            Ok(KeybindingAction::SwitchWorkspace((*workspace).to_owned()))
        }
        ["workspace", "create", workspace] | ["workspace-create", workspace] => {
            Ok(KeybindingAction::CreateWorkspace((*workspace).to_owned()))
        }
        ["workspace", "destroy", workspace] | ["workspace-destroy", workspace] => {
            Ok(KeybindingAction::DestroyWorkspace((*workspace).to_owned()))
        }
        ["output", "enable", output] | ["output-enable", output] => {
            Ok(KeybindingAction::EnableOutput((*output).to_owned()))
        }
        ["output", "disable", output] | ["output-disable", output] => {
            Ok(KeybindingAction::DisableOutput((*output).to_owned()))
        }
        ["output", "configure", output, mode] | ["output-configure", output, mode] => {
            Ok(KeybindingAction::ConfigureOutput {
                output: (*output).to_owned(),
                mode: (*mode).to_owned(),
                scale: None,
            })
        }
        ["output", "configure", output, mode, scale]
        | ["output-configure", output, mode, scale] => Ok(KeybindingAction::ConfigureOutput {
            output: (*output).to_owned(),
            mode: (*mode).to_owned(),
            scale: Some(parse_u32("output scale", scale)?),
        }),
        ["spawn-terminal"] => Ok(KeybindingAction::LaunchTerminal),
        ["launcher"] | ["show-launcher"] => Ok(KeybindingAction::LaunchApplicationLauncher),
        ["show-power-menu"] | ["power-menu"] => Ok(KeybindingAction::ShowPowerMenu),
        ["exec", rest @ ..] if !rest.is_empty() => {
            Ok(KeybindingAction::Exec(rest.iter().map(|part| (*part).to_owned()).collect()))
        }
        _ => Err(format!("unsupported action `{command}`")),
    }
}

fn dispatch_keybinding_action(
    binding: &CompiledKeybinding,
    command_config: &ExternalCommandConfig,
    keyboard_focus: &KeyboardFocusState,
    pending_input_events: &mut PendingInputEvents,
    pending_external_commands: &mut PendingExternalCommandRequests,
    pending_window_requests: &mut PendingWindowServerRequests,
    pending_workspace_requests: &mut PendingWorkspaceServerRequests,
    pending_output_requests: &mut PendingOutputServerRequests,
) {
    match &binding.action {
        KeybindingAction::CloseFocusedWindow => {
            let Some(surface_id) = focused_surface(keyboard_focus, binding, pending_input_events)
            else {
                return;
            };

            pending_window_requests
                .items
                .push(WindowServerRequest { surface_id, action: WindowServerAction::Close });
        }
        KeybindingAction::MoveFocusedWindow { x, y } => {
            let Some(surface_id) = focused_surface(keyboard_focus, binding, pending_input_events)
            else {
                return;
            };

            pending_window_requests.items.push(WindowServerRequest {
                surface_id,
                action: WindowServerAction::Move { x: *x, y: *y },
            });
        }
        KeybindingAction::ResizeFocusedWindow { width, height } => {
            let Some(surface_id) = focused_surface(keyboard_focus, binding, pending_input_events)
            else {
                return;
            };

            pending_window_requests.items.push(WindowServerRequest {
                surface_id,
                action: WindowServerAction::Resize { width: *width, height: *height },
            });
        }
        KeybindingAction::SwitchWorkspace(workspace) => {
            pending_workspace_requests.items.push(WorkspaceServerRequest {
                action: WorkspaceServerAction::Switch { workspace: workspace.clone() },
            });
        }
        KeybindingAction::CreateWorkspace(workspace) => {
            pending_workspace_requests.items.push(WorkspaceServerRequest {
                action: WorkspaceServerAction::Create { workspace: workspace.clone() },
            });
        }
        KeybindingAction::DestroyWorkspace(workspace) => {
            pending_workspace_requests.items.push(WorkspaceServerRequest {
                action: WorkspaceServerAction::Destroy { workspace: workspace.clone() },
            });
        }
        KeybindingAction::EnableOutput(output) => {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Enable { output: output.clone() },
            });
        }
        KeybindingAction::DisableOutput(output) => {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Disable { output: output.clone() },
            });
        }
        KeybindingAction::ConfigureOutput { output, mode, scale } => {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Configure {
                    output: output.clone(),
                    mode: mode.clone(),
                    scale: *scale,
                },
            });
        }
        KeybindingAction::LaunchTerminal => {
            commands::queue_external_command(
                format!("{} -> {}", binding.binding, binding.command),
                ExternalCommandKind::Terminal,
                command_config,
                pending_external_commands,
            );
        }
        KeybindingAction::LaunchApplicationLauncher => {
            commands::queue_external_command(
                format!("{} -> {}", binding.binding, binding.command),
                ExternalCommandKind::Launcher,
                command_config,
                pending_external_commands,
            );
        }
        KeybindingAction::ShowPowerMenu => {
            commands::queue_external_command(
                format!("{} -> {}", binding.binding, binding.command),
                ExternalCommandKind::PowerMenu,
                command_config,
                pending_external_commands,
            );
        }
        KeybindingAction::Exec(argv) => {
            commands::queue_exec_command(
                format!("{} -> {}", binding.binding, binding.command),
                argv.clone(),
                pending_external_commands,
            );
        }
    }

    pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
        source: "keybinding".to_owned(),
        detail: format!("{} -> {}", binding.binding, binding.command),
    });
}

fn focused_surface(
    keyboard_focus: &KeyboardFocusState,
    binding: &CompiledKeybinding,
    pending_input_events: &mut PendingInputEvents,
) -> Option<u64> {
    if let Some(surface_id) = keyboard_focus.focused_surface {
        return Some(surface_id);
    }

    pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
        source: "keybinding".to_owned(),
        detail: format!("{} -> {} ignored: no focused surface", binding.binding, binding.command),
    });
    None
}

fn normalize_modifier_name(token: &str) -> Option<&'static str> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some("ctrl"),
        "alt" => Some("alt"),
        "shift" => Some("shift"),
        "super" | "logo" | "meta" => Some("logo"),
        _ => None,
    }
}

fn parse_keycode(token: &str) -> Result<u32, String> {
    let normalized = token.to_ascii_lowercase();
    match normalized.as_str() {
        "1" => Ok(10),
        "2" => Ok(11),
        "3" => Ok(12),
        "4" => Ok(13),
        "5" => Ok(14),
        "6" => Ok(15),
        "7" => Ok(16),
        "8" => Ok(17),
        "9" => Ok(18),
        "0" => Ok(19),
        "q" => Ok(24),
        "w" => Ok(25),
        "e" => Ok(26),
        "r" => Ok(27),
        "t" => Ok(28),
        "y" => Ok(29),
        "u" => Ok(30),
        "i" => Ok(31),
        "o" => Ok(32),
        "p" => Ok(33),
        "a" => Ok(38),
        "s" => Ok(39),
        "d" => Ok(40),
        "f" => Ok(41),
        "g" => Ok(42),
        "h" => Ok(43),
        "j" => Ok(44),
        "k" => Ok(45),
        "l" => Ok(46),
        "z" => Ok(52),
        "x" => Ok(53),
        "c" => Ok(54),
        "v" => Ok(55),
        "b" => Ok(56),
        "n" => Ok(57),
        "m" => Ok(58),
        "tab" => Ok(23),
        "return" | "enter" => Ok(36),
        "space" => Ok(65),
        "escape" | "esc" => Ok(9),
        "backspace" => Ok(22),
        "delete" => Ok(119),
        "left" => Ok(113),
        "right" => Ok(114),
        "up" => Ok(111),
        "down" => Ok(116),
        "f1" => Ok(67),
        "f2" => Ok(68),
        "f3" => Ok(69),
        "f4" => Ok(70),
        "f5" => Ok(71),
        "f6" => Ok(72),
        "f7" => Ok(73),
        "f8" => Ok(74),
        "f9" => Ok(75),
        "f10" => Ok(76),
        "f11" => Ok(95),
        "f12" => Ok(96),
        _ => Err(format!("unsupported key `{token}`")),
    }
}

fn parse_i32(label: &str, value: &str) -> Result<i32, String> {
    value.parse::<i32>().map_err(|error| format!("invalid {label}: {error}"))
}

fn parse_u32(label: &str, value: &str) -> Result<u32, String> {
    value.parse::<u32>().map_err(|error| format!("invalid {label}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use bevy_ecs::message::MessageReader;
    use bevy_ecs::prelude::{ResMut, Resource};
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::InputSchedule;
    use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CommandExecutionStatus, CommandHistoryState,
        CompositorClock, CompositorConfig, ExternalCommandConfig, ExternalCommandRequest,
        KeyboardFocusState, PendingBackendInputEvents, PendingExternalCommandRequests,
        PendingInputEvents, PendingOutputServerRequests, PendingWindowServerRequests,
        PendingWorkspaceServerRequests, WindowServerAction, WorkspaceServerAction,
    };
    use nekoland_protocol::{ProtocolServerState, XWaylandServerState};

    use crate::{InputPlugin, commands};

    use super::{
        CompiledKeybinding, ExternalCommandKind, KeybindingAction, KeybindingEngine,
        compile_keybinding, dispatch_keybinding_action, parse_keybinding_action,
    };

    const SUPER_KEYCODE: u32 = 133;
    const Q_KEYCODE: u32 = 24;
    const W_KEYCODE: u32 = 25;
    const TWO_KEYCODE: u32 = 11;

    #[derive(Debug, Default, Resource)]
    struct TestAudit;

    #[derive(Debug, Default, Resource)]
    struct ExternalCommandAudit {
        launched: Vec<ExternalCommandLaunched>,
        failed: Vec<ExternalCommandFailed>,
    }

    #[test]
    fn configured_keybindings_queue_control_plane_requests() {
        let mut app = test_app(config_with_bindings([
            ("Super+Q", "close-window"),
            ("Super+2", "workspace 2"),
        ]));

        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface = Some(77);
        app.inner_mut().insert_resource(PendingBackendInputEvents {
            items: vec![
                key_event(SUPER_KEYCODE, true),
                key_event(Q_KEYCODE, true),
                key_event(TWO_KEYCODE, true),
            ],
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let window_requests = world
            .get_resource::<PendingWindowServerRequests>()
            .expect("window request queue should exist");
        let workspace_requests = world
            .get_resource::<PendingWorkspaceServerRequests>()
            .expect("workspace request queue should exist");
        let output_requests = world
            .get_resource::<PendingOutputServerRequests>()
            .expect("output request queue should exist");
        let external_commands = world
            .get_resource::<PendingExternalCommandRequests>()
            .expect("external command queue should exist");
        let engine =
            world.get_resource::<KeybindingEngine>().expect("keybinding engine should exist");

        assert_eq!(
            window_requests.items,
            vec![nekoland_ecs::resources::WindowServerRequest {
                surface_id: 77,
                action: WindowServerAction::Close,
            }]
        );
        assert_eq!(
            workspace_requests.items,
            vec![nekoland_ecs::resources::WorkspaceServerRequest {
                action: WorkspaceServerAction::Switch { workspace: "2".to_owned() },
            }]
        );
        assert!(output_requests.items.is_empty(), "no output binding should have been triggered");
        assert!(
            external_commands.items.is_empty(),
            "close/workspace keybindings should not queue external commands"
        );
        assert_eq!(engine.bindings_loaded, 2);
    }

    #[test]
    fn keybinding_engine_reloads_when_bindings_change_without_length_change() {
        let mut app = test_app(config_with_bindings([("Super+Q", "close-window")]));

        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface = Some(42);
        app.inner_mut().insert_resource(PendingBackendInputEvents {
            items: vec![key_event(SUPER_KEYCODE, true), key_event(Q_KEYCODE, true)],
        });
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        {
            let world = app.inner_mut().world_mut();
            world.resource_mut::<PendingWindowServerRequests>().items.clear();
            world.resource_mut::<CompositorConfig>().keybindings =
                config_with_bindings([("Super+W", "workspace switch 3")]).keybindings;
            world.insert_resource(PendingBackendInputEvents {
                items: vec![
                    key_event(SUPER_KEYCODE, true),
                    key_event(Q_KEYCODE, true),
                    key_event(W_KEYCODE, true),
                ],
            });
        }

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let window_requests = world
            .get_resource::<PendingWindowServerRequests>()
            .expect("window request queue should exist");
        let workspace_requests = world
            .get_resource::<PendingWorkspaceServerRequests>()
            .expect("workspace request queue should exist");

        assert!(
            window_requests.items.is_empty(),
            "stale binding should not survive a same-length config reload"
        );
        assert_eq!(
            workspace_requests.items,
            vec![nekoland_ecs::resources::WorkspaceServerRequest {
                action: WorkspaceServerAction::Switch { workspace: "3".to_owned() },
            }]
        );
    }

    #[test]
    fn parser_supports_launcher_and_workspace_aliases() {
        assert_eq!(
            parse_keybinding_action("launcher"),
            Ok(KeybindingAction::LaunchApplicationLauncher)
        );
        assert_eq!(parse_keybinding_action("show-power-menu"), Ok(KeybindingAction::ShowPowerMenu));
        assert_eq!(
            parse_keybinding_action("workspace 9"),
            Ok(KeybindingAction::SwitchWorkspace("9".to_owned()))
        );
        assert_eq!(
            parse_keybinding_action("exec foot --server"),
            Ok(KeybindingAction::Exec(vec!["foot".to_owned(), "--server".to_owned()]))
        );
    }

    #[test]
    fn split_command_line_respects_quotes_and_escapes() {
        assert_eq!(
            commands::split_command_line("wofi --prompt 'Pick one' --style=\"dark mode.css\""),
            vec![
                "wofi".to_owned(),
                "--prompt".to_owned(),
                "Pick one".to_owned(),
                "--style=dark mode.css".to_owned(),
            ]
        );
    }

    #[test]
    fn launcher_and_power_menu_bindings_queue_external_commands() {
        let launcher_binding =
            compile_keybinding("Super+Space", "launcher").expect("launcher binding should compile");
        let power_binding = compile_keybinding("Super+P", "show-power-menu")
            .expect("power menu binding should compile");
        let command_config = ExternalCommandConfig {
            terminal: None,
            launcher: Some("rofi -show drun".to_owned()),
            power_menu: Some("wlogout --protocol layer-shell".to_owned()),
        };

        let mut pending_input_events = PendingInputEvents::default();
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_requests = PendingWindowServerRequests::default();
        let mut pending_workspace_requests = PendingWorkspaceServerRequests::default();
        let mut pending_output_requests = PendingOutputServerRequests::default();
        let keyboard_focus = KeyboardFocusState::default();

        queue_external_binding(
            &launcher_binding,
            &command_config,
            &keyboard_focus,
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut pending_window_requests,
            &mut pending_workspace_requests,
            &mut pending_output_requests,
        );
        queue_external_binding(
            &power_binding,
            &command_config,
            &keyboard_focus,
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut pending_window_requests,
            &mut pending_workspace_requests,
            &mut pending_output_requests,
        );

        assert!(pending_window_requests.items.is_empty());
        assert!(pending_workspace_requests.items.is_empty());
        assert!(pending_output_requests.items.is_empty());
        assert_eq!(pending_external_commands.items.len(), 2);
        assert_eq!(
            pending_external_commands.items[0].candidates[0],
            vec!["rofi".to_owned(), "-show".to_owned(), "drun".to_owned()]
        );
        assert_eq!(
            pending_external_commands.items[1].candidates[0],
            vec!["wlogout".to_owned(), "--protocol".to_owned(), "layer-shell".to_owned()]
        );
    }

    #[test]
    fn launcher_and_power_menu_have_distinct_fallback_candidates() {
        assert_eq!(
            commands::command_candidates(
                ExternalCommandKind::Launcher,
                &ExternalCommandConfig::default(),
            )[0],
            vec!["fuzzel".to_owned()]
        );
        assert_eq!(
            commands::command_candidates(
                ExternalCommandKind::PowerMenu,
                &ExternalCommandConfig::default(),
            )[0],
            vec!["wlogout".to_owned()]
        );
    }

    #[test]
    fn configured_external_commands_are_preferred_to_fallback_candidates() {
        let command_config = ExternalCommandConfig {
            terminal: None,
            launcher: Some("walker --modules applications".to_owned()),
            power_menu: None,
        };
        let candidates =
            commands::command_candidates(ExternalCommandKind::Launcher, &command_config);
        assert_eq!(
            candidates[0],
            vec!["walker".to_owned(), "--modules".to_owned(), "applications".to_owned()]
        );
        assert_eq!(candidates[1], vec!["fuzzel".to_owned()]);
    }

    #[test]
    fn external_command_launch_system_emits_launch_messages() {
        let mut app = test_app(CompositorConfig::default());
        app.inner_mut().init_resource::<ExternalCommandAudit>().add_systems(
            InputSchedule,
            capture_external_command_messages.after(commands::external_command_launch_system),
        );
        app.inner_mut().insert_resource(PendingExternalCommandRequests {
            items: vec![ExternalCommandRequest {
                origin: "test launch".to_owned(),
                candidates: vec![vec!["true".to_owned()]],
            }],
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let audit = world
            .get_resource::<ExternalCommandAudit>()
            .expect("external command audit should exist");

        assert_eq!(audit.launched.len(), 1);
        assert!(audit.failed.is_empty());
        assert_eq!(audit.launched[0].origin, "test launch");
        assert_eq!(audit.launched[0].command, vec!["true".to_owned()]);
    }

    #[test]
    fn external_command_launch_system_emits_failure_messages() {
        let mut app = test_app(CompositorConfig::default());
        app.inner_mut().init_resource::<ExternalCommandAudit>().add_systems(
            InputSchedule,
            capture_external_command_messages.after(commands::external_command_launch_system),
        );
        app.inner_mut().insert_resource(PendingExternalCommandRequests {
            items: vec![ExternalCommandRequest {
                origin: "test fail".to_owned(),
                candidates: vec![vec!["/definitely-not-a-real-nekoland-command".to_owned()]],
            }],
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let audit = world
            .get_resource::<ExternalCommandAudit>()
            .expect("external command audit should exist");
        let pending_input_events =
            world.get_resource::<PendingInputEvents>().expect("input event audit should exist");

        assert!(audit.launched.is_empty());
        assert_eq!(audit.failed.len(), 1);
        assert_eq!(audit.failed[0].origin, "test fail");
        assert_eq!(
            audit.failed[0].candidates,
            vec![vec!["/definitely-not-a-real-nekoland-command".to_owned()]]
        );
        assert!(
            audit.failed[0].error.contains("No such file")
                || audit.failed[0].error.contains("not found"),
            "failure should retain the last spawn error: {:?}",
            audit.failed[0]
        );
        assert!(
            pending_input_events.items.iter().any(|event| event.detail.contains("test fail")),
            "failure should still be mirrored into the input audit log"
        );
    }

    #[test]
    fn startup_commands_queue_once_after_wayland_socket_is_ready() {
        let mut config = CompositorConfig::default();
        config.startup_commands = vec!["true".to_owned()];
        let mut app = test_app(config);
        app.inner_mut().insert_resource(ProtocolServerState {
            socket_name: Some("wayland-77".to_owned()),
            runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
            ..ProtocolServerState::default()
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let history =
            world.get_resource::<CommandHistoryState>().expect("command history should exist");
        let startup_state = world
            .get_resource::<commands::StartupCommandState>()
            .expect("startup state should exist");

        assert!(startup_state.queued, "startup commands should be marked as queued");
        assert_eq!(history.items.len(), 1, "startup commands should only run once");
        assert_eq!(history.items[0].origin, "startup -> true");
        assert!(matches!(
            history.items[0].status.as_ref(),
            Some(CommandExecutionStatus::Launched { .. })
        ));
    }

    #[test]
    fn startup_commands_can_be_disabled_via_env() {
        let _env_lock = env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("NEKOLAND_DISABLE_STARTUP_COMMANDS");
        unsafe {
            std::env::set_var("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
        }

        let mut config = CompositorConfig::default();
        config.startup_commands = vec!["true".to_owned()];
        let mut app = test_app(config);
        app.inner_mut().insert_resource(ProtocolServerState {
            socket_name: Some("wayland-77".to_owned()),
            runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
            ..ProtocolServerState::default()
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let history =
            world.get_resource::<CommandHistoryState>().expect("command history should exist");
        let startup_state = world
            .get_resource::<commands::StartupCommandState>()
            .expect("startup state should exist");

        assert!(startup_state.queued, "startup commands should still mark the queue as settled");
        assert!(history.items.is_empty(), "disabled startup commands should not execute");

        match previous {
            Some(previous) => unsafe {
                std::env::set_var("NEKOLAND_DISABLE_STARTUP_COMMANDS", previous);
            },
            None => unsafe {
                std::env::remove_var("NEKOLAND_DISABLE_STARTUP_COMMANDS");
            },
        }
    }

    #[test]
    fn external_command_launch_injects_nested_wayland_environment() {
        let _env_lock = env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let runtime_dir = unique_temp_dir("nested-env-runtime");
        let output_path = runtime_dir.join("env.txt");
        let script_path = runtime_dir.join("print-env");

        fs::create_dir_all(&runtime_dir).expect("test runtime directory should be created");
        fs::write(
            &script_path,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n%s\\n' \"$WAYLAND_DISPLAY\" \"$XDG_RUNTIME_DIR\" > \"{}\"\n",
                output_path.display()
            ),
        )
        .expect("test env printer script should be written");
        let mut permissions =
            fs::metadata(&script_path).expect("script metadata should exist").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions)
            .expect("test env printer script should be executable");

        let mut app = test_app(CompositorConfig::default());
        app.inner_mut().insert_resource(ProtocolServerState {
            socket_name: Some("wayland-55".to_owned()),
            runtime_dir: Some(runtime_dir.to_string_lossy().into_owned()),
            ..ProtocolServerState::default()
        });
        app.inner_mut().insert_resource(PendingExternalCommandRequests {
            items: vec![ExternalCommandRequest {
                origin: "nested env".to_owned(),
                candidates: vec![vec![
                    script_path.to_string_lossy().into_owned(),
                    output_path.to_string_lossy().into_owned(),
                ]],
            }],
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let output = wait_for_file_contents(&output_path);
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines, vec!["wayland-55", runtime_dir.to_string_lossy().as_ref()]);

        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_dir_all(&runtime_dir);
    }

    #[test]
    fn external_command_launch_injects_xwayland_display_when_ready() {
        let _env_lock = env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let runtime_dir = unique_temp_dir("nested-env-runtime-x11");
        let output_path = runtime_dir.join("env.txt");
        let script_path = runtime_dir.join("print-env");

        fs::create_dir_all(&runtime_dir).expect("test runtime directory should be created");
        fs::write(
            &script_path,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n%s\\n%s\\n' \"$WAYLAND_DISPLAY\" \"$XDG_RUNTIME_DIR\" \"$DISPLAY\" > \"{}\"\n",
                output_path.display()
            ),
        )
        .expect("test env printer script should be written");
        let mut permissions =
            fs::metadata(&script_path).expect("script metadata should exist").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions)
            .expect("test env printer script should be executable");

        let mut app = test_app(CompositorConfig::default());
        app.inner_mut().insert_resource(ProtocolServerState {
            socket_name: Some("wayland-55".to_owned()),
            runtime_dir: Some(runtime_dir.to_string_lossy().into_owned()),
            ..ProtocolServerState::default()
        });
        app.inner_mut().insert_resource(XWaylandServerState {
            enabled: true,
            ready: true,
            display_number: Some(88),
            display_name: Some(":88".to_owned()),
            ..XWaylandServerState::default()
        });
        app.inner_mut().insert_resource(PendingExternalCommandRequests {
            items: vec![ExternalCommandRequest {
                origin: "nested env xwayland".to_owned(),
                candidates: vec![vec![
                    script_path.to_string_lossy().into_owned(),
                    output_path.to_string_lossy().into_owned(),
                ]],
            }],
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let output = wait_for_file_contents(&output_path);
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines, vec!["wayland-55", runtime_dir.to_string_lossy().as_ref(), ":88"]);

        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_dir_all(&runtime_dir);
    }

    #[test]
    fn command_history_limit_tracks_runtime_config_and_trims_existing_records() {
        let mut config = CompositorConfig::default();
        config.command_history_limit = 1;
        let mut app = test_app(config);

        app.inner_mut().insert_resource(PendingExternalCommandRequests {
            items: vec![ExternalCommandRequest {
                origin: "launch".to_owned(),
                candidates: vec![vec!["true".to_owned()]],
            }],
        });
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        app.inner_mut().insert_resource(PendingExternalCommandRequests {
            items: vec![ExternalCommandRequest {
                origin: "fail".to_owned(),
                candidates: vec![vec!["/definitely-not-a-real-nekoland-command".to_owned()]],
            }],
        });
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        {
            let world = app.inner().world();
            let history =
                world.get_resource::<CommandHistoryState>().expect("command history should exist");

            assert_eq!(history.limit, 1);
            assert_eq!(history.items.len(), 1);
            assert_eq!(history.items[0].origin, "fail");
            assert!(matches!(
                history.items[0].status.as_ref(),
                Some(CommandExecutionStatus::Failed { .. })
            ));
        }

        app.inner_mut().world_mut().resource_mut::<CompositorConfig>().command_history_limit = 0;
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let history =
            world.get_resource::<CommandHistoryState>().expect("command history should exist");
        assert_eq!(history.limit, 0);
        assert!(
            history.items.is_empty(),
            "disabling command history should clear retained records"
        );
    }

    fn test_app(config: CompositorConfig) -> NekolandApp {
        let mut app = NekolandApp::new("input-keybindings-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(config)
            .add_plugin(InputPlugin);
        app.inner_mut().init_resource::<TestAudit>();
        app
    }

    fn config_with_bindings<const N: usize>(bindings: [(&str, &str); N]) -> CompositorConfig {
        let mut config = CompositorConfig::default();
        config.keybindings = bindings
            .into_iter()
            .map(|(binding, command)| (binding.to_owned(), command.to_owned()))
            .collect::<BTreeMap<_, _>>();
        config
    }

    fn key_event(keycode: u32, pressed: bool) -> BackendInputEvent {
        BackendInputEvent {
            device: "test".to_owned(),
            action: BackendInputAction::Key { keycode, pressed },
        }
    }

    fn queue_external_binding(
        binding: &CompiledKeybinding,
        command_config: &ExternalCommandConfig,
        keyboard_focus: &KeyboardFocusState,
        pending_input_events: &mut PendingInputEvents,
        pending_external_commands: &mut PendingExternalCommandRequests,
        pending_window_requests: &mut PendingWindowServerRequests,
        pending_workspace_requests: &mut PendingWorkspaceServerRequests,
        pending_output_requests: &mut PendingOutputServerRequests,
    ) {
        dispatch_keybinding_action(
            binding,
            command_config,
            keyboard_focus,
            pending_input_events,
            pending_external_commands,
            pending_window_requests,
            pending_workspace_requests,
            pending_output_requests,
        );
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn capture_external_command_messages(
        mut launched: MessageReader<ExternalCommandLaunched>,
        mut failed: MessageReader<ExternalCommandFailed>,
        mut audit: ResMut<ExternalCommandAudit>,
    ) {
        audit.launched.extend(launched.read().cloned());
        audit.failed.extend(failed.read().cloned());
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("wall clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("nekoland-{label}-{}-{nanos}", std::process::id()))
    }

    fn wait_for_file_contents(path: &PathBuf) -> String {
        for _ in 0..50 {
            if let Ok(contents) = fs::read_to_string(path) {
                return contents;
            }
            thread::sleep(Duration::from_millis(20));
        }

        panic!("timed out waiting for {}", path.display());
    }
}
