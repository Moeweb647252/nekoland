use std::collections::BTreeMap;

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::control::{
    OutputControlApi, OutputOps, WindowControlApi, WindowOps, WorkspaceControlApi, WorkspaceOps,
};
use nekoland_ecs::events::KeyPress;
use nekoland_ecs::resources::{
    CompositorConfig, ConfiguredKeybindingAction, ModifierState, PendingExternalCommandRequests,
    PendingInputEvents, SplitAxis,
};
use nekoland_ecs::selectors::{OutputName, WorkspaceLookup, WorkspaceName, WorkspaceSelector};
use nekoland_shell::commands;

/// Holds the compiled keybinding table derived from the latest compositor config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct KeybindingEngine {
    pub bindings_loaded: usize,
    loaded_bindings: BTreeMap<String, ConfiguredKeybindingAction>,
    compiled_bindings: Vec<CompiledKeybinding>,
}

/// Reloads keybindings when config changes and dispatches pressed keys into the corresponding
/// pending request queues.
pub fn keybinding_dispatch_system(
    config: Res<CompositorConfig>,
    modifiers: Res<ModifierState>,
    mut engine: ResMut<KeybindingEngine>,
    mut key_events: MessageReader<KeyPress>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
    mut windows: WindowOps,
    mut workspaces: WorkspaceOps,
    mut outputs: OutputOps,
) {
    if engine.loaded_bindings != config.keybindings {
        engine.reload_bindings(&config.keybindings, &mut pending_input_events);
    }

    let mut observed = 0_usize;
    let modifiers = modifiers.into_inner();
    let mut window_controls = windows.api();
    let mut workspace_controls = workspaces.api();
    let mut output_controls = outputs.api();
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
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut window_controls,
            &mut workspace_controls,
            &mut output_controls,
        );
    }

    tracing::trace!(observed, bindings_loaded = engine.bindings_loaded, "keybinding dispatch tick");
}

/// Internal compiled representation of one config keybinding.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledKeybinding {
    chord: KeyChord,
    action: KeybindingAction,
    binding: String,
    action_label: String,
}

/// Modifier/keycode tuple used for exact binding matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyChord {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
    keycode: u32,
}

/// Normalized action space for keybindings after parsing config syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeybindingAction {
    CloseFocusedWindow,
    MoveFocusedWindow { x: i32, y: i32 },
    ResizeFocusedWindow { width: u32, height: u32 },
    SplitFocusedWindow { axis: SplitAxis },
    SwitchWorkspace(String),
    CreateWorkspace(String),
    DestroyWorkspace(String),
    EnableOutput(String),
    DisableOutput(String),
    ConfigureOutput { output: String, mode: String, scale: Option<u32> },
    Exec(Vec<String>),
}

impl KeybindingEngine {
    /// Rebuilds the compiled table from config and records invalid bindings in the input event log
    /// instead of failing the whole reload.
    fn reload_bindings(
        &mut self,
        bindings: &BTreeMap<String, ConfiguredKeybindingAction>,
        pending_input_events: &mut PendingInputEvents,
    ) {
        self.loaded_bindings = bindings.clone();
        self.compiled_bindings.clear();

        for (binding, configured_action) in bindings {
            match compile_keybinding(binding, configured_action) {
                Ok(compiled) => self.compiled_bindings.push(compiled),
                Err(error) => {
                    let action = configured_action.describe();
                    tracing::warn!(binding, action, error, "ignoring invalid keybinding");
                    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                        source: "keybinding".to_owned(),
                        detail: format!("{binding} -> {action} ignored: {error}"),
                    });
                }
            }
        }

        self.bindings_loaded = self.compiled_bindings.len();
    }

    /// Finds the first binding whose chord exactly matches the current key press plus modifier
    /// snapshot.
    fn match_binding(
        &self,
        keycode: u32,
        modifiers: &ModifierState,
    ) -> Option<&CompiledKeybinding> {
        self.compiled_bindings.iter().find(|binding| binding.chord.matches(keycode, modifiers))
    }
}

impl KeyChord {
    /// Matching is strict on every modifier bit so `Super+Q` and `Super+Shift+Q` remain distinct.
    fn matches(&self, keycode: u32, modifiers: &ModifierState) -> bool {
        self.keycode == keycode
            && self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
            && self.logo == modifiers.logo
    }
}

/// Parses and compiles one config keybinding entry.
fn compile_keybinding(
    binding: &str,
    configured_action: &ConfiguredKeybindingAction,
) -> Result<CompiledKeybinding, String> {
    Ok(CompiledKeybinding {
        chord: parse_key_chord(binding)?,
        action: parse_keybinding_action(configured_action)?,
        binding: binding.to_owned(),
        action_label: configured_action.describe(),
    })
}

/// Converts textual chords like `Super+Shift+Q` into the internal modifier/keycode form.
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

/// Normalizes config actions into the runtime action enum used by the dispatcher.
fn parse_keybinding_action(
    configured_action: &ConfiguredKeybindingAction,
) -> Result<KeybindingAction, String> {
    match configured_action {
        ConfiguredKeybindingAction::Action(command) => parse_builtin_keybinding_action(command),
        ConfiguredKeybindingAction::Command(argv) => {
            let Some(program) = argv.first() else {
                return Err("command binding must include at least one argv element".to_owned());
            };
            if program.trim().is_empty() {
                return Err("command binding must not start with an empty program".to_owned());
            }
            Ok(KeybindingAction::Exec(argv.clone()))
        }
    }
}

/// Parses the string form used by legacy built-in actions.
fn parse_builtin_keybinding_action(command: &str) -> Result<KeybindingAction, String> {
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
        ["window", "split", axis] => {
            Ok(KeybindingAction::SplitFocusedWindow { axis: parse_split_axis(axis)? })
        }
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
        _ => Err(format!("unsupported action `{command}`")),
    }
}

/// Emits the concrete side effect associated with one compiled binding.
fn dispatch_keybinding_action(
    binding: &CompiledKeybinding,
    pending_input_events: &mut PendingInputEvents,
    pending_external_commands: &mut PendingExternalCommandRequests,
    windows: &mut WindowControlApi<'_>,
    workspaces: &mut WorkspaceControlApi<'_>,
    outputs: &mut OutputControlApi<'_>,
) {
    match &binding.action {
        KeybindingAction::CloseFocusedWindow => {
            let Some(mut window) = focused_window(binding, pending_input_events, windows) else {
                return;
            };

            window.close();
        }
        KeybindingAction::MoveFocusedWindow { x, y } => {
            let Some(mut window) = focused_window(binding, pending_input_events, windows) else {
                return;
            };

            window.move_to(*x, *y);
        }
        KeybindingAction::ResizeFocusedWindow { width, height } => {
            let Some(mut window) = focused_window(binding, pending_input_events, windows) else {
                return;
            };

            window.resize_to(*width, *height);
        }
        KeybindingAction::SplitFocusedWindow { axis } => {
            let Some(mut window) = focused_window(binding, pending_input_events, windows) else {
                return;
            };

            window.split(*axis);
        }
        KeybindingAction::SwitchWorkspace(workspace) => {
            workspaces.switch_or_create(WorkspaceLookup::parse(workspace));
        }
        KeybindingAction::CreateWorkspace(workspace) => {
            workspaces.create_named(WorkspaceName::from(workspace.clone()));
        }
        KeybindingAction::DestroyWorkspace(workspace) => {
            workspaces.destroy(WorkspaceSelector::parse(workspace));
        }
        KeybindingAction::EnableOutput(output) => {
            outputs.named(OutputName::from(output.clone())).enable();
        }
        KeybindingAction::DisableOutput(output) => {
            outputs.named(OutputName::from(output.clone())).disable();
        }
        KeybindingAction::ConfigureOutput { output, mode, scale } => {
            outputs.named(OutputName::from(output.clone())).configure(mode.clone(), *scale);
        }
        KeybindingAction::Exec(argv) => {
            commands::queue_exec_command(
                format!("{} -> {}", binding.binding, binding.action_label),
                argv.clone(),
                pending_external_commands,
            );
        }
    }

    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: "keybinding".to_owned(),
        detail: format!("{} -> {}", binding.binding, binding.action_label),
    });
}

fn focused_window<'a>(
    binding: &CompiledKeybinding,
    pending_input_events: &mut PendingInputEvents,
    windows: &'a mut WindowControlApi<'_>,
) -> Option<nekoland_ecs::resources::WindowControlHandle<'a>> {
    if let Some(window) = windows.focused() {
        return Some(window);
    }

    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: "keybinding".to_owned(),
        detail: format!(
            "{} -> {} ignored: no focused surface",
            binding.binding, binding.action_label
        ),
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

fn parse_split_axis(token: &str) -> Result<SplitAxis, String> {
    match token.to_ascii_lowercase().as_str() {
        "horizontal" | "h" | "x" => Ok(SplitAxis::Horizontal),
        "vertical" | "v" | "y" => Ok(SplitAxis::Vertical),
        _ => Err(format!("unsupported split axis `{token}`")),
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
    use nekoland_core::schedules::{InputSchedule, LayoutSchedule};
    use nekoland_ecs::components::WorkspaceId;
    use nekoland_ecs::control::{OutputControlApi, WindowControlApi, WorkspaceControlApi};
    use nekoland_ecs::events::{ExternalCommandFailed, ExternalCommandLaunched};
    use nekoland_ecs::resources::SplitAxis;
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CommandExecutionStatus, CommandHistoryState,
        CompositorClock, CompositorConfig, ConfiguredKeybindingAction, ExternalCommandRequest,
        KeyboardFocusState, PendingBackendInputEvents, PendingExternalCommandRequests,
        PendingInputEvents, PendingOutputControls, PendingWindowControls, PendingWorkspaceControls,
        WorkspaceControl,
    };
    use nekoland_ecs::selectors::{SurfaceId, WorkspaceLookup};
    use nekoland_protocol::{ProtocolServerState, XWaylandServerState};
    use nekoland_shell::commands;

    use crate::InputPlugin;

    use super::{
        CompiledKeybinding, KeybindingAction, KeybindingEngine, compile_keybinding,
        dispatch_keybinding_action, parse_keybinding_action,
    };

    const SUPER_KEYCODE: u32 = 133;
    const Q_KEYCODE: u32 = 24;
    const S_KEYCODE: u32 = 39;
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
            ("Super+Q", builtin("close-window")),
            ("Super+S", builtin("window split vertical")),
            ("Super+2", builtin("workspace 2")),
        ]));

        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface = Some(77);
        app.inner_mut().insert_resource(PendingBackendInputEvents::from_items(vec![
            key_event(SUPER_KEYCODE, true),
            key_event(Q_KEYCODE, true),
            key_event(S_KEYCODE, true),
            key_event(TWO_KEYCODE, true),
        ]));

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let window_controls = world
            .get_resource::<PendingWindowControls>()
            .expect("window control queue should exist");
        let workspace_controls = world
            .get_resource::<PendingWorkspaceControls>()
            .expect("workspace control queue should exist");
        let output_controls = world
            .get_resource::<PendingOutputControls>()
            .expect("output control queue should exist");
        let external_commands = world
            .get_resource::<PendingExternalCommandRequests>()
            .expect("external command queue should exist");
        let engine =
            world.get_resource::<KeybindingEngine>().expect("keybinding engine should exist");

        assert_eq!(
            window_controls.as_slice(),
            [nekoland_ecs::resources::PendingWindowControl {
                surface_id: SurfaceId(77),
                position: None,
                size: None,
                split_axis: Some(SplitAxis::Vertical),
                focus: false,
                close: true,
            }]
        );
        assert_eq!(
            workspace_controls.as_slice(),
            [WorkspaceControl::SwitchOrCreate { target: WorkspaceLookup::Id(WorkspaceId(2)) }]
        );
        assert!(output_controls.is_empty(), "no output binding should have been triggered");
        assert!(
            external_commands.is_empty(),
            "close/workspace keybindings should not queue external commands"
        );
        assert_eq!(engine.bindings_loaded, 3);
    }

    #[test]
    fn keybinding_engine_reloads_when_bindings_change_without_length_change() {
        let mut app = test_app(config_with_bindings([("Super+Q", builtin("close-window"))]));

        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface = Some(42);
        app.inner_mut().insert_resource(PendingBackendInputEvents::from_items(vec![
            key_event(SUPER_KEYCODE, true),
            key_event(Q_KEYCODE, true),
        ]));
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        {
            let world = app.inner_mut().world_mut();
            world.resource_mut::<PendingWindowControls>().clear();
            world.resource_mut::<CompositorConfig>().keybindings =
                config_with_bindings([("Super+W", builtin("workspace switch 3"))]).keybindings;
            world.insert_resource(PendingBackendInputEvents::from_items(vec![
                key_event(SUPER_KEYCODE, true),
                key_event(Q_KEYCODE, true),
                key_event(W_KEYCODE, true),
            ]));
        }

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let window_controls = world
            .get_resource::<PendingWindowControls>()
            .expect("window control queue should exist");
        let workspace_controls = world
            .get_resource::<PendingWorkspaceControls>()
            .expect("workspace control queue should exist");

        assert!(
            window_controls.is_empty(),
            "stale binding should not survive a same-length config reload"
        );
        assert_eq!(
            workspace_controls.as_slice(),
            [WorkspaceControl::SwitchOrCreate { target: WorkspaceLookup::Id(WorkspaceId(3)) }]
        );
    }

    #[test]
    fn parser_supports_workspace_actions_and_command_argv() {
        assert_eq!(
            parse_keybinding_action(&builtin("workspace 9")),
            Ok(KeybindingAction::SwitchWorkspace("9".to_owned()))
        );
        assert_eq!(
            parse_keybinding_action(&builtin("window split vertical")),
            Ok(KeybindingAction::SplitFocusedWindow { axis: SplitAxis::Vertical })
        );
        assert_eq!(
            parse_keybinding_action(&argv(["foot", "--server"])),
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
    fn command_argv_bindings_queue_external_commands() {
        let launcher_binding = compile_keybinding("Super+Space", &argv(["rofi", "-show", "drun"]))
            .expect("launcher binding should compile");
        let power_binding =
            compile_keybinding("Super+P", &argv(["wlogout", "--protocol", "layer-shell"]))
                .expect("power menu binding should compile");

        let mut pending_input_events = PendingInputEvents::default();
        let mut pending_external_commands = PendingExternalCommandRequests::default();
        let mut pending_window_controls = PendingWindowControls::default();
        let mut pending_workspace_controls = PendingWorkspaceControls::default();
        let mut pending_output_controls = PendingOutputControls::default();
        let keyboard_focus = KeyboardFocusState::default();

        queue_external_binding(
            &launcher_binding,
            &keyboard_focus,
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut pending_window_controls,
            &mut pending_workspace_controls,
            &mut pending_output_controls,
        );
        queue_external_binding(
            &power_binding,
            &keyboard_focus,
            &mut pending_input_events,
            &mut pending_external_commands,
            &mut pending_window_controls,
            &mut pending_workspace_controls,
            &mut pending_output_controls,
        );

        assert!(pending_window_controls.is_empty());
        assert!(pending_workspace_controls.is_empty());
        assert!(pending_output_controls.is_empty());
        assert_eq!(pending_external_commands.len(), 2);
        assert_eq!(
            pending_external_commands.as_slice()[0].candidates[0],
            vec!["rofi".to_owned(), "-show".to_owned(), "drun".to_owned()]
        );
        assert_eq!(
            pending_external_commands.as_slice()[1].candidates[0],
            vec!["wlogout".to_owned(), "--protocol".to_owned(), "layer-shell".to_owned()]
        );
    }

    #[test]
    fn external_command_launch_system_emits_launch_messages() {
        let mut app = test_app_with_commands(CompositorConfig::default());
        app.inner_mut().init_resource::<ExternalCommandAudit>().add_systems(
            LayoutSchedule,
            capture_external_command_messages.after(commands::external_command_launch_system),
        );
        app.inner_mut().insert_resource(PendingExternalCommandRequests::from_items(vec![
            ExternalCommandRequest {
                origin: "test launch".to_owned(),
                candidates: vec![vec!["true".to_owned()]],
            },
        ]));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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
        let mut app = test_app_with_commands(CompositorConfig::default());
        app.inner_mut().init_resource::<ExternalCommandAudit>().add_systems(
            LayoutSchedule,
            capture_external_command_messages.after(commands::external_command_launch_system),
        );
        app.inner_mut().insert_resource(PendingExternalCommandRequests::from_items(vec![
            ExternalCommandRequest {
                origin: "test fail".to_owned(),
                candidates: vec![vec!["/definitely-not-a-real-nekoland-command".to_owned()]],
            },
        ]));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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
            pending_input_events.iter().any(|event| event.detail.contains("test fail")),
            "failure should still be mirrored into the input audit log"
        );
    }

    #[test]
    fn startup_commands_queue_once_after_wayland_socket_is_ready() {
        let mut config = CompositorConfig::default();
        config.startup_commands = vec!["true".to_owned()];
        let mut app = test_app_with_commands(config);
        app.inner_mut().init_resource::<commands::StartupCommandState>().insert_resource(
            ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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
    fn startup_commands_wait_for_xwayland_ready_when_enabled() {
        let mut config = CompositorConfig::default();
        config.startup_commands = vec!["true".to_owned()];
        let mut app = test_app_with_commands(config);
        app.inner_mut()
            .init_resource::<commands::StartupCommandState>()
            .insert_resource(ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            })
            .insert_resource(XWaylandServerState {
                enabled: true,
                ready: false,
                ..XWaylandServerState::default()
            });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let history =
            world.get_resource::<CommandHistoryState>().expect("command history should exist");
        let startup_state = world
            .get_resource::<commands::StartupCommandState>()
            .expect("startup state should exist");

        assert!(!startup_state.queued, "startup commands should wait for xwayland ready");
        assert!(history.items.is_empty(), "no commands should have been launched yet");

        app.inner_mut().world_mut().resource_mut::<XWaylandServerState>().ready = true;
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let history =
            world.get_resource::<CommandHistoryState>().expect("command history should exist");
        let startup_state = world
            .get_resource::<commands::StartupCommandState>()
            .expect("startup state should exist");

        assert!(startup_state.queued, "startup commands should be queued after xwayland ready");
        assert_eq!(history.items.len(), 1, "startup commands should run after xwayland ready");
    }

    #[test]
    fn startup_commands_run_immediately_when_xwayland_disabled() {
        let mut config = CompositorConfig::default();
        config.startup_commands = vec!["true".to_owned()];
        let mut app = test_app_with_commands(config);
        app.inner_mut()
            .init_resource::<commands::StartupCommandState>()
            .insert_resource(ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            })
            .insert_resource(XWaylandServerState {
                enabled: false,
                ready: false,
                ..XWaylandServerState::default()
            });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let history =
            world.get_resource::<CommandHistoryState>().expect("command history should exist");
        let startup_state = world
            .get_resource::<commands::StartupCommandState>()
            .expect("startup state should exist");

        assert!(startup_state.queued, "startup commands should run when xwayland is disabled");
        assert_eq!(history.items.len(), 1, "startup commands should have been launched");
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
        let mut app = test_app_with_commands(config);
        app.inner_mut().init_resource::<commands::StartupCommandState>().insert_resource(
            ProtocolServerState {
                socket_name: Some("wayland-77".to_owned()),
                runtime_dir: Some("/tmp/nekoland-runtime".to_owned()),
                ..ProtocolServerState::default()
            },
        );

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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

        let mut app = test_app_with_commands(CompositorConfig::default());
        app.inner_mut().insert_resource(ProtocolServerState {
            socket_name: Some("wayland-55".to_owned()),
            runtime_dir: Some(runtime_dir.to_string_lossy().into_owned()),
            ..ProtocolServerState::default()
        });
        app.inner_mut().insert_resource(PendingExternalCommandRequests::from_items(vec![
            ExternalCommandRequest {
                origin: "nested env".to_owned(),
                candidates: vec![vec![
                    script_path.to_string_lossy().into_owned(),
                    output_path.to_string_lossy().into_owned(),
                ]],
            },
        ]));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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

        let mut app = test_app_with_commands(CompositorConfig::default());
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
        app.inner_mut().insert_resource(PendingExternalCommandRequests::from_items(vec![
            ExternalCommandRequest {
                origin: "nested env xwayland".to_owned(),
                candidates: vec![vec![
                    script_path.to_string_lossy().into_owned(),
                    output_path.to_string_lossy().into_owned(),
                ]],
            },
        ]));

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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
        let mut app = test_app_with_commands(config);

        app.inner_mut().insert_resource(PendingExternalCommandRequests::from_items(vec![
            ExternalCommandRequest {
                origin: "launch".to_owned(),
                candidates: vec![vec!["true".to_owned()]],
            },
        ]));
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        app.inner_mut().insert_resource(PendingExternalCommandRequests::from_items(vec![
            ExternalCommandRequest {
                origin: "fail".to_owned(),
                candidates: vec![vec!["/definitely-not-a-real-nekoland-command".to_owned()]],
            },
        ]));
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

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

    fn test_app_with_commands(config: CompositorConfig) -> NekolandApp {
        let mut app = NekolandApp::new("input-keybindings-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(config)
            .add_plugin(InputPlugin);
        app.inner_mut()
            .init_resource::<commands::StartupCommandState>()
            .init_resource::<CommandHistoryState>()
            .init_resource::<PendingExternalCommandRequests>()
            .init_resource::<PendingInputEvents>()
            .add_message::<ExternalCommandLaunched>()
            .add_message::<ExternalCommandFailed>()
            .add_systems(
                LayoutSchedule,
                (
                    commands::startup_command_queue_system,
                    commands::external_command_launch_system,
                    commands::command_history_system,
                )
                    .chain(),
            );
        app.inner_mut().init_resource::<TestAudit>();
        app
    }

    fn config_with_bindings<const N: usize>(
        bindings: [(&str, ConfiguredKeybindingAction); N],
    ) -> CompositorConfig {
        let mut config = CompositorConfig::default();
        config.keybindings = bindings
            .into_iter()
            .map(|(binding, action)| (binding.to_owned(), action))
            .collect::<BTreeMap<_, _>>();
        config
    }

    fn builtin(action: &str) -> ConfiguredKeybindingAction {
        ConfiguredKeybindingAction::Action(action.to_owned())
    }

    fn argv<const N: usize>(parts: [&str; N]) -> ConfiguredKeybindingAction {
        ConfiguredKeybindingAction::Command(
            parts.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        )
    }

    fn key_event(keycode: u32, pressed: bool) -> BackendInputEvent {
        BackendInputEvent {
            device: "test".to_owned(),
            action: BackendInputAction::Key { keycode, pressed },
        }
    }

    fn queue_external_binding(
        binding: &CompiledKeybinding,
        keyboard_focus: &KeyboardFocusState,
        pending_input_events: &mut PendingInputEvents,
        pending_external_commands: &mut PendingExternalCommandRequests,
        pending_window_controls: &mut PendingWindowControls,
        pending_workspace_controls: &mut PendingWorkspaceControls,
        pending_output_controls: &mut PendingOutputControls,
    ) {
        let mut windows = WindowControlApi::new(keyboard_focus, pending_window_controls);
        let mut workspaces = WorkspaceControlApi::new(pending_workspace_controls);
        let mut outputs = OutputControlApi::new(pending_output_controls);
        dispatch_keybinding_action(
            binding,
            pending_input_events,
            pending_external_commands,
            &mut windows,
            &mut workspaces,
            &mut outputs,
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
