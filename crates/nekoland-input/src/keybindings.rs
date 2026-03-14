use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::control::{OutputOps, WindowOps, WorkspaceOps};
use nekoland_ecs::resources::{
    CompositorConfig, ConfiguredAction, KeyShortcut, PendingExternalCommandRequests,
    PendingInputEvents, PressedKeys,
};
use nekoland_ecs::selectors::{OutputName, WorkspaceLookup};

/// Feature-local compiled keybindings derived from the latest compositor config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub struct CompiledKeybindings {
    loaded_bindings: BTreeMap<String, Vec<ConfiguredAction>>,
    window_bindings: Vec<WindowKeybinding>,
    workspace_bindings: Vec<WorkspaceKeybinding>,
    output_bindings: Vec<OutputKeybinding>,
    command_bindings: Vec<CommandKeybinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowKeybinding {
    shortcut: KeyShortcut,
    binding: String,
    action: WindowKeybindingAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WindowKeybindingAction {
    CloseFocused,
    MoveFocused { x: isize, y: isize },
    ResizeFocused { width: u32, height: u32 },
    SplitFocused { axis: nekoland_ecs::resources::SplitAxis },
    BackgroundFocused { output: OutputName },
    ClearFocusedBackground,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceKeybinding {
    shortcut: KeyShortcut,
    binding: String,
    action: WorkspaceKeybindingAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceKeybindingAction {
    Switch { workspace: WorkspaceLookup },
    Create { workspace: WorkspaceLookup },
    Destroy { workspace: nekoland_ecs::selectors::WorkspaceSelector },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutputKeybinding {
    shortcut: KeyShortcut,
    binding: String,
    action: OutputKeybindingAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputKeybindingAction {
    Enable { output: OutputName },
    Disable { output: OutputName },
    Configure { output: OutputName, mode: String, scale: Option<u32> },
    PanViewport { delta_x: isize, delta_y: isize },
    MoveViewport { x: isize, y: isize },
    CenterViewportOnFocusedWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandKeybinding {
    shortcut: KeyShortcut,
    binding: String,
    argv: Vec<String>,
}

pub fn reload_keybindings_system(
    config: Res<CompositorConfig>,
    mut compiled: ResMut<CompiledKeybindings>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    if compiled.loaded_bindings == config.keybindings {
        return;
    }

    compiled.loaded_bindings = config.keybindings.clone();
    compiled.window_bindings.clear();
    compiled.workspace_bindings.clear();
    compiled.output_bindings.clear();
    compiled.command_bindings.clear();

    for (binding, actions) in &config.keybindings {
        match compile_binding(binding, actions) {
            Ok(binding_set) => {
                compiled.window_bindings.extend(binding_set.window_bindings);
                compiled.workspace_bindings.extend(binding_set.workspace_bindings);
                compiled.output_bindings.extend(binding_set.output_bindings);
                compiled.command_bindings.extend(binding_set.command_bindings);
            }
            Err(error) => {
                let action = nekoland_ecs::resources::describe_action_sequence(actions);
                tracing::warn!(binding, action, error, "ignoring invalid keybinding");
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: "keybinding".to_owned(),
                    detail: format!("{binding} -> {action} ignored: {error}"),
                });
            }
        }
    }
}

pub fn window_keybinding_system(
    pressed_keys: Res<PressedKeys>,
    bindings: Res<CompiledKeybindings>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut windows: WindowOps,
) {
    for binding in &bindings.window_bindings {
        if !pressed_keys.just_pressed(&binding.shortcut) {
            continue;
        }

        let mut window_controls = windows.api();
        match &binding.action {
            WindowKeybindingAction::CloseFocused => {
                let Some(mut window) = window_controls.focused() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        "close-focused-window",
                        "no focused surface",
                    );
                    continue;
                };
                window.close();
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    "close-focused-window",
                );
            }
            WindowKeybindingAction::MoveFocused { x, y } => {
                let Some(mut window) = window_controls.focused() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        &format!("move-focused-window {x} {y}"),
                        "no focused surface",
                    );
                    continue;
                };
                window.move_to(*x, *y);
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("move-focused-window {x} {y}"),
                );
            }
            WindowKeybindingAction::ResizeFocused { width, height } => {
                let Some(mut window) = window_controls.focused() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        &format!("resize-focused-window {width} {height}"),
                        "no focused surface",
                    );
                    continue;
                };
                window.resize_to(*width, *height);
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("resize-focused-window {width} {height}"),
                );
            }
            WindowKeybindingAction::SplitFocused { axis } => {
                let Some(mut window) = window_controls.focused() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        &format!("split-focused-window {}", split_axis_label(*axis)),
                        "no focused surface",
                    );
                    continue;
                };
                window.split(*axis);
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("split-focused-window {}", split_axis_label(*axis)),
                );
            }
            WindowKeybindingAction::BackgroundFocused { output } => {
                let Some(mut window) = window_controls.focused() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        &format!("background-focused-window {}", output.as_str()),
                        "no focused surface",
                    );
                    continue;
                };
                window.background_on(output.clone());
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("background-focused-window {}", output.as_str()),
                );
            }
            WindowKeybindingAction::ClearFocusedBackground => {
                let Some(mut window) = window_controls.focused() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        "clear-focused-window-background",
                        "no focused surface",
                    );
                    continue;
                };
                window.clear_background();
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    "clear-focused-window-background",
                );
            }
        }
    }
}

pub fn workspace_keybinding_system(
    pressed_keys: Res<PressedKeys>,
    bindings: Res<CompiledKeybindings>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut workspaces: WorkspaceOps,
) {
    for binding in &bindings.workspace_bindings {
        if !pressed_keys.just_pressed(&binding.shortcut) {
            continue;
        }

        match &binding.action {
            WorkspaceKeybindingAction::Switch { workspace } => {
                workspaces.switch_or_create(workspace.clone());
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("switch-workspace {}", workspace_lookup_label(workspace)),
                );
            }
            WorkspaceKeybindingAction::Create { workspace } => {
                workspaces.create(workspace.clone());
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("create-workspace {}", workspace_lookup_label(workspace)),
                );
            }
            WorkspaceKeybindingAction::Destroy { workspace } => {
                workspaces.destroy(workspace.clone());
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("destroy-workspace {}", workspace_selector_label(workspace)),
                );
            }
        }
    }
}

pub fn output_keybinding_system(
    pressed_keys: Res<PressedKeys>,
    bindings: Res<CompiledKeybindings>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    windows: WindowOps,
    mut outputs: OutputOps,
) {
    for binding in &bindings.output_bindings {
        if !pressed_keys.just_pressed(&binding.shortcut) {
            continue;
        }

        match &binding.action {
            OutputKeybindingAction::Enable { output } => {
                outputs.named(output.clone()).enable();
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("enable-output {}", output.as_str()),
                );
            }
            OutputKeybindingAction::Disable { output } => {
                outputs.named(output.clone()).disable();
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("disable-output {}", output.as_str()),
                );
            }
            OutputKeybindingAction::Configure { output, mode, scale } => {
                outputs.named(output.clone()).configure(mode.clone(), *scale);
                let description = match scale {
                    Some(scale) => format!("configure-output {} {mode} {scale}", output.as_str()),
                    None => format!("configure-output {} {mode}", output.as_str()),
                };
                log_keybinding_applied(&mut pending_input_events, &binding.binding, &description);
            }
            OutputKeybindingAction::PanViewport { delta_x, delta_y } => {
                outputs.focused().pan_viewport_by(*delta_x, *delta_y);
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("pan-viewport {delta_x} {delta_y}"),
                );
            }
            OutputKeybindingAction::MoveViewport { x, y } => {
                outputs.focused().move_viewport_to(*x, *y);
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    &format!("move-viewport {x} {y}"),
                );
            }
            OutputKeybindingAction::CenterViewportOnFocusedWindow => {
                let Some(surface_id) = windows.focused_surface_id() else {
                    log_keybinding_ignored(
                        &mut pending_input_events,
                        &binding.binding,
                        "center-viewport-on-focused-window",
                        "no focused surface",
                    );
                    continue;
                };
                outputs.focused().center_viewport_on_window(surface_id);
                log_keybinding_applied(
                    &mut pending_input_events,
                    &binding.binding,
                    "center-viewport-on-focused-window",
                );
            }
        }
    }
}

pub fn command_keybinding_system(
    pressed_keys: Res<PressedKeys>,
    bindings: Res<CompiledKeybindings>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut pending_external_commands: ResMut<PendingExternalCommandRequests>,
) {
    for binding in &bindings.command_bindings {
        if !pressed_keys.just_pressed(&binding.shortcut) {
            continue;
        }

        pending_external_commands.push(nekoland_ecs::resources::ExternalCommandRequest {
            origin: format!("{} -> {}", binding.binding, binding.argv.join(" ")),
            candidates: vec![binding.argv.clone()],
        });
        log_keybinding_applied(
            &mut pending_input_events,
            &binding.binding,
            &binding.argv.join(" "),
        );
    }
}

#[derive(Default)]
struct CompiledBindingSet {
    window_bindings: Vec<WindowKeybinding>,
    workspace_bindings: Vec<WorkspaceKeybinding>,
    output_bindings: Vec<OutputKeybinding>,
    command_bindings: Vec<CommandKeybinding>,
}

fn compile_binding(
    binding: &str,
    configured_actions: &[ConfiguredAction],
) -> Result<CompiledBindingSet, String> {
    if configured_actions.is_empty() {
        return Err("action sequence must contain at least one action".to_owned());
    }

    let shortcut = parse_key_shortcut(binding)?;
    let mut compiled = CompiledBindingSet::default();

    for action in configured_actions {
        match action {
            ConfiguredAction::Exec { argv } => {
                let Some(program) = argv.first() else {
                    return Err("command action must include at least one argv element".to_owned());
                };
                if program.trim().is_empty() {
                    return Err("command action must not start with an empty program".to_owned());
                }
                compiled.command_bindings.push(CommandKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    argv: argv.clone(),
                });
            }
            ConfiguredAction::CloseFocusedWindow => {
                compiled.window_bindings.push(WindowKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WindowKeybindingAction::CloseFocused,
                });
            }
            ConfiguredAction::MoveFocusedWindow { x, y } => {
                compiled.window_bindings.push(WindowKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WindowKeybindingAction::MoveFocused { x: *x, y: *y },
                });
            }
            ConfiguredAction::ResizeFocusedWindow { width, height } => {
                compiled.window_bindings.push(WindowKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WindowKeybindingAction::ResizeFocused {
                        width: *width,
                        height: *height,
                    },
                });
            }
            ConfiguredAction::SplitFocusedWindow { axis } => {
                compiled.window_bindings.push(WindowKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WindowKeybindingAction::SplitFocused { axis: *axis },
                });
            }
            ConfiguredAction::BackgroundFocusedWindow { output } => {
                compiled.window_bindings.push(WindowKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WindowKeybindingAction::BackgroundFocused { output: output.clone() },
                });
            }
            ConfiguredAction::ClearFocusedWindowBackground => {
                compiled.window_bindings.push(WindowKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WindowKeybindingAction::ClearFocusedBackground,
                });
            }
            ConfiguredAction::SwitchWorkspace { workspace } => {
                compiled.workspace_bindings.push(WorkspaceKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WorkspaceKeybindingAction::Switch { workspace: workspace.clone() },
                });
            }
            ConfiguredAction::CreateWorkspace { workspace } => {
                compiled.workspace_bindings.push(WorkspaceKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WorkspaceKeybindingAction::Create { workspace: workspace.clone() },
                });
            }
            ConfiguredAction::DestroyWorkspace { workspace } => {
                compiled.workspace_bindings.push(WorkspaceKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: WorkspaceKeybindingAction::Destroy { workspace: workspace.clone() },
                });
            }
            ConfiguredAction::EnableOutput { output } => {
                compiled.output_bindings.push(OutputKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: OutputKeybindingAction::Enable { output: output.clone() },
                });
            }
            ConfiguredAction::DisableOutput { output } => {
                compiled.output_bindings.push(OutputKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: OutputKeybindingAction::Disable { output: output.clone() },
                });
            }
            ConfiguredAction::ConfigureOutput { output, mode, scale } => {
                compiled.output_bindings.push(OutputKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: OutputKeybindingAction::Configure {
                        output: output.clone(),
                        mode: mode.clone(),
                        scale: *scale,
                    },
                });
            }
            ConfiguredAction::PanViewport { delta_x, delta_y } => {
                compiled.output_bindings.push(OutputKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: OutputKeybindingAction::PanViewport {
                        delta_x: *delta_x,
                        delta_y: *delta_y,
                    },
                });
            }
            ConfiguredAction::MoveViewport { x, y } => {
                compiled.output_bindings.push(OutputKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: OutputKeybindingAction::MoveViewport { x: *x, y: *y },
                });
            }
            ConfiguredAction::CenterViewportOnFocusedWindow => {
                compiled.output_bindings.push(OutputKeybinding {
                    shortcut: shortcut.clone(),
                    binding: binding.to_owned(),
                    action: OutputKeybindingAction::CenterViewportOnFocusedWindow,
                });
            }
        }
    }

    Ok(compiled)
}

fn log_keybinding_applied(
    pending_input_events: &mut PendingInputEvents,
    binding: &str,
    description: &str,
) {
    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: "keybinding".to_owned(),
        detail: format!("{binding} -> {description}"),
    });
}

fn log_keybinding_ignored(
    pending_input_events: &mut PendingInputEvents,
    binding: &str,
    description: &str,
    reason: &str,
) {
    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: "keybinding".to_owned(),
        detail: format!("{binding} -> {description} ignored: {reason}"),
    });
}

pub(crate) fn parse_key_shortcut(binding: &str) -> Result<KeyShortcut, String> {
    let mut modifiers = nekoland_ecs::resources::ModifierMask::default();
    let mut keycode = None;

    for token in binding.split('+').map(str::trim).filter(|token| !token.is_empty()) {
        match normalize_modifier_name(token) {
            Some("ctrl") => modifiers.ctrl = true,
            Some("alt") => modifiers.alt = true,
            Some("shift") => modifiers.shift = true,
            Some("logo") => modifiers.logo = true,
            Some(_) => unreachable!(),
            None => {
                if keycode.is_some() {
                    return Err("binding must contain exactly one non-modifier key".to_owned());
                }
                keycode = Some(parse_keycode(token)?);
            }
        }
    }

    let keycode = keycode.ok_or_else(|| "binding is missing a non-modifier key".to_owned())?;
    Ok(KeyShortcut::new(modifiers, Some(keycode)))
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

fn split_axis_label(axis: nekoland_ecs::resources::SplitAxis) -> &'static str {
    match axis {
        nekoland_ecs::resources::SplitAxis::Horizontal => "horizontal",
        nekoland_ecs::resources::SplitAxis::Vertical => "vertical",
    }
}

fn workspace_lookup_label(workspace: &WorkspaceLookup) -> String {
    match workspace {
        WorkspaceLookup::Id(id) => id.0.to_string(),
        WorkspaceLookup::Name(name) => name.as_str().to_owned(),
    }
}

fn workspace_selector_label(workspace: &nekoland_ecs::selectors::WorkspaceSelector) -> String {
    match workspace {
        nekoland_ecs::selectors::WorkspaceSelector::Active => "active".to_owned(),
        nekoland_ecs::selectors::WorkspaceSelector::Id(id) => id.0.to_string(),
        nekoland_ecs::selectors::WorkspaceSelector::Name(name) => name.as_str().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::InputSchedule;
    use nekoland_ecs::components::WorkspaceId;
    use nekoland_ecs::resources::SplitAxis;
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CompositorClock, CompositorConfig, ConfiguredAction,
        KeyboardFocusState, PendingBackendInputEvents, PendingExternalCommandRequests,
        PendingOutputControls, PendingWindowControls, PendingWorkspaceControls, WorkspaceControl,
    };
    use nekoland_ecs::selectors::{OutputName, SurfaceId, WorkspaceLookup};

    use crate::InputPlugin;

    use super::{CompiledKeybindings, compile_binding, parse_key_shortcut};

    const SUPER_KEYCODE: u32 = 133;
    const Q_KEYCODE: u32 = 24;
    const H_KEYCODE: u32 = 43;
    const B_KEYCODE: u32 = 56;
    const S_KEYCODE: u32 = 39;
    const W_KEYCODE: u32 = 25;
    const TWO_KEYCODE: u32 = 11;
    const RETURN_KEYCODE: u32 = 36;

    #[test]
    fn configured_keybindings_queue_control_plane_requests() {
        let mut app = test_app(config_with_bindings([
            ("Super+Q", close_focused_window()),
            ("Super+S", split_focused_window(SplitAxis::Vertical)),
            ("Super+2", switch_workspace("2")),
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
        let compiled =
            world.get_resource::<CompiledKeybindings>().expect("compiled keybindings should exist");

        assert_eq!(
            window_controls.as_slice(),
            [nekoland_ecs::resources::PendingWindowControl {
                surface_id: SurfaceId(77),
                position: None,
                size: None,
                split_axis: Some(SplitAxis::Vertical),
                background: None,
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
        assert_eq!(compiled.window_bindings.len(), 2);
        assert_eq!(compiled.workspace_bindings.len(), 1);
    }

    #[test]
    fn viewport_keybinding_queues_focused_output_controls() {
        let mut app = test_app(config_with_bindings([("Super+H", pan_viewport(-40, 25))]));

        app.inner_mut().insert_resource(PendingBackendInputEvents::from_items(vec![
            key_event(SUPER_KEYCODE, true),
            key_event(H_KEYCODE, true),
        ]));

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let output_controls =
            app.inner().world().get_resource::<PendingOutputControls>().expect("output controls");
        assert_eq!(
            output_controls.as_slice(),
            &[nekoland_ecs::resources::PendingOutputControl {
                selector: nekoland_ecs::selectors::OutputSelector::Focused,
                enabled: None,
                configuration: None,
                viewport_origin: None,
                viewport_pan: Some(nekoland_ecs::resources::OutputViewportPan {
                    delta_x: -40,
                    delta_y: 25,
                }),
                center_viewport_on: None,
            }]
        );
    }

    #[test]
    fn background_keybinding_queues_window_background_control() {
        let mut app =
            test_app(config_with_bindings([("Super+B", background_focused_window("Virtual-1"))]));
        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface = Some(77);
        app.inner_mut().insert_resource(PendingBackendInputEvents::from_items(vec![
            key_event(SUPER_KEYCODE, true),
            key_event(B_KEYCODE, true),
        ]));

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let window_controls =
            app.inner().world().get_resource::<PendingWindowControls>().expect("window controls");
        assert_eq!(window_controls.as_slice().len(), 1);
        assert!(matches!(
            window_controls.as_slice()[0].background,
            Some(nekoland_ecs::resources::WindowBackgroundControl::Set { ref output })
                if output.as_str() == "Virtual-1"
        ));
    }

    #[test]
    fn command_keybindings_queue_external_commands() {
        let mut app =
            test_app(config_with_bindings([("Super+Return", exec(["foot", "--server"]))]));
        app.inner_mut().insert_resource(PendingBackendInputEvents::from_items(vec![
            key_event(SUPER_KEYCODE, true),
            key_event(RETURN_KEYCODE, true),
        ]));

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let external_commands = app
            .inner()
            .world()
            .get_resource::<PendingExternalCommandRequests>()
            .expect("external command queue should exist");
        assert_eq!(external_commands.len(), 1);
        assert_eq!(
            external_commands.as_slice()[0].candidates[0],
            vec!["foot".to_owned(), "--server".to_owned()]
        );
    }

    #[test]
    fn compiled_keybindings_reload_when_bindings_change_without_length_change() {
        let mut app = test_app(config_with_bindings([("Super+Q", close_focused_window())]));

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
                config_with_bindings([("Super+W", switch_workspace("3"))]).keybindings;
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
    fn parse_key_shortcut_requires_exactly_one_non_modifier_key() {
        assert_eq!(
            parse_key_shortcut("Super+Alt"),
            Err("binding is missing a non-modifier key".to_owned())
        );
        assert_eq!(
            parse_key_shortcut("Super+Alt+H+J"),
            Err("binding must contain exactly one non-modifier key".to_owned())
        );
    }

    #[test]
    fn compile_binding_rejects_invalid_exec_argv() {
        assert_eq!(
            compile_binding("Super+Return", &[ConfiguredAction::Exec { argv: vec![] }]).map(|_| ()),
            Err("command action must include at least one argv element".to_owned())
        );
    }

    fn test_app(config: CompositorConfig) -> NekolandApp {
        let mut app = NekolandApp::new("input-keybindings-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(config)
            .add_plugin(InputPlugin);
        app
    }

    fn config_with_bindings<const N: usize>(
        bindings: [(&str, ConfiguredAction); N],
    ) -> CompositorConfig {
        let mut config = CompositorConfig::default();
        config.keybindings = bindings
            .into_iter()
            .map(|(binding, action)| (binding.to_owned(), vec![action]))
            .collect::<BTreeMap<_, _>>();
        config
    }

    fn close_focused_window() -> ConfiguredAction {
        ConfiguredAction::CloseFocusedWindow
    }

    fn split_focused_window(axis: SplitAxis) -> ConfiguredAction {
        ConfiguredAction::SplitFocusedWindow { axis }
    }

    fn switch_workspace(workspace: &str) -> ConfiguredAction {
        ConfiguredAction::SwitchWorkspace { workspace: WorkspaceLookup::parse(workspace) }
    }

    fn pan_viewport(delta_x: isize, delta_y: isize) -> ConfiguredAction {
        ConfiguredAction::PanViewport { delta_x, delta_y }
    }

    fn background_focused_window(output: &str) -> ConfiguredAction {
        ConfiguredAction::BackgroundFocusedWindow { output: OutputName::from(output) }
    }

    fn exec<const N: usize>(parts: [&str; N]) -> ConfiguredAction {
        ConfiguredAction::Exec { argv: parts.into_iter().map(str::to_owned).collect::<Vec<_>>() }
    }

    fn key_event(keycode: u32, pressed: bool) -> BackendInputEvent {
        BackendInputEvent {
            device: "test".to_owned(),
            action: BackendInputAction::Key { keycode, pressed },
        }
    }
}
