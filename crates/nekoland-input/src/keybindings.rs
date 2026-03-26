//! Shortcut registry compilation and runtime matching.

use std::collections::BTreeMap;

use bevy_ecs::change_detection::DetectChanges;
use bevy_ecs::prelude::{Res, ResMut};
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::resources::{
    CompiledShortcut, CompiledShortcutMap, KeyShortcut, PressedKeys, ShortcutCompileDiagnostics,
    ShortcutMatchState, ShortcutRegistry, ShortcutState, ShortcutTrigger,
};

/// Rebuilds compiled shortcuts from the global registry plus config overrides.
///
/// On compile failure the previous compiled shortcut map stays live and the latest error is
/// surfaced through [`ShortcutCompileDiagnostics`].
pub fn shortcut_compile_system(
    config: Res<'_, CompositorConfig>,
    registry: Res<'_, ShortcutRegistry>,
    mut compiled: ResMut<'_, CompiledShortcutMap>,
    mut diagnostics: ResMut<'_, ShortcutCompileDiagnostics>,
) {
    if !compiled.is_empty() && !config.is_changed() && !registry.is_changed() {
        return;
    }

    match compile_shortcuts(&registry, &config.keybindings) {
        Ok(next) => {
            compiled.replace(next);
            diagnostics.last_error = None;
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to compile configured shortcuts");
            diagnostics.last_error = Some(error);
        }
    }
}

/// Samples the current pressed-key snapshot into per-shortcut runtime activation state.
pub fn shortcut_match_system(
    pressed_keys: Res<'_, PressedKeys>,
    compiled: Res<'_, CompiledShortcutMap>,
    mut shortcut_state: ResMut<'_, ShortcutState>,
) {
    let previous = shortcut_state.clone();
    let mut next = BTreeMap::new();

    for shortcut in compiled.iter() {
        let previous_active = previous.get(&shortcut.id).is_some_and(|state| state.active);
        let active = shortcut_active(&pressed_keys, &shortcut.combo);
        let (just_pressed, just_released) = match shortcut.trigger {
            ShortcutTrigger::Press => {
                (press_triggered(&pressed_keys, &shortcut.combo, previous_active), false)
            }
            ShortcutTrigger::Release => {
                (false, release_triggered(&pressed_keys, &shortcut.combo, previous_active))
            }
            ShortcutTrigger::Hold => (active && !previous_active, !active && previous_active),
        };
        next.insert(
            shortcut.id.clone(),
            ShortcutMatchState { active, just_pressed, just_released },
        );
    }

    shortcut_state.replace(next);
}

fn compile_shortcuts(
    registry: &ShortcutRegistry,
    overrides: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, CompiledShortcut>, String> {
    for shortcut_id in overrides.keys() {
        if registry.get(shortcut_id).is_none() {
            return Err(format!("unknown shortcut id `{shortcut_id}`"));
        }
    }

    let mut compiled = BTreeMap::new();
    let mut occupied =
        BTreeMap::<(u8, bool, bool, bool, bool, Option<u32>), String>::new();

    for spec in registry.iter() {
        let (binding, overridden) = overrides
            .get(&spec.id)
            .map(|binding| (binding.clone(), true))
            .unwrap_or_else(|| (spec.default_binding.clone(), false));
        let combo = KeyShortcut::parse_config(&binding)
            .map_err(|error| format!("shortcut `{}` failed to compile: {error}", spec.id))?;
        let conflict_key = (
            trigger_order(spec.trigger),
            combo.modifiers.ctrl,
            combo.modifiers.alt,
            combo.modifiers.shift,
            combo.modifiers.logo,
            combo.keycode,
        );
        if let Some(previous_id) = occupied.insert(conflict_key, spec.id.clone()) {
            return Err(format!(
                "shortcut `{}` conflicts with `{previous_id}` on binding `{binding}`",
                spec.id
            ));
        }

        compiled.insert(
            spec.id.clone(),
            CompiledShortcut {
                id: spec.id.clone(),
                owner: spec.owner.clone(),
                description: spec.description.clone(),
                binding,
                combo,
                trigger: spec.trigger,
                overridden,
            },
        );
    }

    Ok(compiled)
}

fn shortcut_active(pressed_keys: &PressedKeys, combo: &KeyShortcut) -> bool {
    if combo.keycode.is_none() {
        combo.modifiers.matches_required(pressed_keys.modifiers())
    } else {
        pressed_keys.is_pressed(combo)
    }
}

fn press_triggered(
    pressed_keys: &PressedKeys,
    combo: &KeyShortcut,
    previous_active: bool,
) -> bool {
    if combo.keycode.is_none() {
        shortcut_active(pressed_keys, combo) && !previous_active
    } else {
        pressed_keys.just_pressed(combo)
    }
}

fn release_triggered(
    pressed_keys: &PressedKeys,
    combo: &KeyShortcut,
    previous_active: bool,
) -> bool {
    if combo.keycode.is_none() {
        !shortcut_active(pressed_keys, combo) && previous_active
    } else {
        pressed_keys.just_released(combo)
    }
}

fn trigger_order(trigger: ShortcutTrigger) -> u8 {
    match trigger {
        ShortcutTrigger::Press => 0,
        ShortcutTrigger::Release => 1,
        ShortcutTrigger::Hold => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_config::resources::CompositorConfig;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::InputSchedule;
    use nekoland_ecs::resources::{
        CompiledShortcutMap, PressedKeys, ShortcutCompileDiagnostics, ShortcutRegistry,
        ShortcutState, ShortcutTrigger,
    };

    use super::{compile_shortcuts, shortcut_compile_system, shortcut_match_system};

    fn registry() -> ShortcutRegistry {
        let mut registry = ShortcutRegistry::default();
        registry
            .register(nekoland_ecs::resources::ShortcutSpec::new(
                "system.quit",
                "system",
                "Quit compositor",
                "Super+Shift+Q",
                ShortcutTrigger::Press,
            ))
            .expect("register quit");
        registry
            .register(nekoland_ecs::resources::ShortcutSpec::new(
                "viewport.pan_mode",
                "viewport",
                "Hold for viewport pan",
                "Super+Alt",
                ShortcutTrigger::Hold,
            ))
            .expect("register pan");
        registry
    }

    #[test]
    fn compile_shortcuts_applies_overrides_by_shortcut_id() {
        let registry = registry();
        let compiled = compile_shortcuts(
            &registry,
            &BTreeMap::from([("system.quit".to_owned(), "Ctrl+Shift+Q".to_owned())]),
        )
        .expect("compile shortcuts");

        assert_eq!(compiled["system.quit"].binding, "Ctrl+Shift+Q");
        assert!(compiled["system.quit"].overridden);
        assert_eq!(compiled["viewport.pan_mode"].binding, "Super+Alt");
        assert!(!compiled["viewport.pan_mode"].overridden);
    }

    #[test]
    fn compile_shortcuts_rejects_unknown_ids() {
        let registry = registry();
        assert_eq!(
            compile_shortcuts(
                &registry,
                &BTreeMap::from([("unknown.shortcut".to_owned(), "Alt+Tab".to_owned())]),
            ),
            Err("unknown shortcut id `unknown.shortcut`".to_owned())
        );
    }

    #[test]
    fn compile_shortcuts_rejects_duplicate_exact_bindings() {
        let mut registry = registry();
        registry
            .register(nekoland_ecs::resources::ShortcutSpec::new(
                "window_switcher.cycle_next",
                "switcher",
                "Cycle windows",
                "Alt+Tab",
                ShortcutTrigger::Press,
            ))
            .expect("register switcher");
        registry
            .register(nekoland_ecs::resources::ShortcutSpec::new(
                "window_switcher.cycle_prev",
                "switcher",
                "Cycle windows backward",
                "Alt+Shift+Tab",
                ShortcutTrigger::Press,
            ))
            .expect("register switcher reverse");

        assert_eq!(
            compile_shortcuts(
                &registry,
                &BTreeMap::from([(
                    "window_switcher.cycle_prev".to_owned(),
                    "Alt+Tab".to_owned(),
                )]),
            ),
            Err(
                "shortcut `window_switcher.cycle_prev` conflicts with `window_switcher.cycle_next` on binding `Alt+Tab`"
                    .to_owned()
            )
        );
    }

    #[test]
    fn compile_system_keeps_previous_map_on_failure() {
        let mut app = NekolandApp::new("shortcut-compile-test");
        let mut config = CompositorConfig::default();
        config.keybindings.insert("system.quit".to_owned(), "Super+Shift+Q".to_owned());
        app.insert_resource(config)
            .insert_resource(registry())
            .insert_resource(CompiledShortcutMap::default())
            .insert_resource(ShortcutCompileDiagnostics::default())
            .inner_mut()
            .add_systems(InputSchedule, shortcut_compile_system);

        app.inner_mut().world_mut().run_schedule(InputSchedule);
        assert!(app.inner().world().resource::<CompiledShortcutMap>().get("system.quit").is_some());

        app.inner_mut()
            .world_mut()
            .resource_mut::<CompositorConfig>()
            .keybindings
            .insert("unknown.shortcut".to_owned(), "Alt+Tab".to_owned());
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        assert!(app.inner().world().resource::<CompiledShortcutMap>().get("system.quit").is_some());
        assert_eq!(
            app.inner().world().resource::<ShortcutCompileDiagnostics>().last_error,
            Some("unknown shortcut id `unknown.shortcut`".to_owned())
        );
    }

    #[test]
    fn match_system_tracks_modifier_only_hold_shortcuts_with_extra_modifiers() {
        let mut app = NekolandApp::new("shortcut-match-test");
        app.insert_resource(PressedKeys::default())
            .insert_resource(registry())
            .insert_resource(CompositorConfig::default())
            .insert_resource(CompiledShortcutMap::default())
            .insert_resource(ShortcutCompileDiagnostics::default())
            .insert_resource(ShortcutState::default())
            .inner_mut()
            .add_systems(InputSchedule, (shortcut_compile_system, shortcut_match_system).chain());

        {
            let mut pressed = app.inner_mut().world_mut().resource_mut::<PressedKeys>();
            pressed.record_key(133, true);
            pressed.record_key(64, true);
            pressed.record_key(50, true);
        }
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let shortcut_state = app.inner().world().resource::<ShortcutState>();
        assert!(shortcut_state.active("viewport.pan_mode"));
        assert!(shortcut_state.just_pressed("viewport.pan_mode"));
        assert!(!shortcut_state.just_pressed("system.quit"));
    }

    #[test]
    fn match_system_tracks_press_shortcuts_with_exact_modifiers() {
        let mut app = NekolandApp::new("shortcut-press-match-test");
        app.insert_resource(PressedKeys::default())
            .insert_resource(registry())
            .insert_resource(CompositorConfig::default())
            .insert_resource(CompiledShortcutMap::default())
            .insert_resource(ShortcutCompileDiagnostics::default())
            .insert_resource(ShortcutState::default())
            .inner_mut()
            .add_systems(InputSchedule, (shortcut_compile_system, shortcut_match_system).chain());

        {
            let mut pressed = app.inner_mut().world_mut().resource_mut::<PressedKeys>();
            pressed.record_key(133, true);
            pressed.record_key(50, true);
            pressed.record_key(24, true);
        }
        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let shortcut_state = app.inner().world().resource::<ShortcutState>();
        assert!(shortcut_state.just_pressed("system.quit"));
    }
}
