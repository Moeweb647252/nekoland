//! Keyboard focus, modifier, and pressed-key snapshots used by input-driven systems.

#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Current keyboard focus target tracked by shell/input systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardFocusState {
    pub focused_surface: Option<u64>,
}

/// Coarse modifier snapshot derived from backend key events.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

/// Snapshot of currently-held keys plus the transitions that occurred in the current input tick.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PressedKeys {
    held: BTreeSet<u32>,
    just_pressed: BTreeSet<u32>,
    just_released: BTreeSet<u32>,
    modifiers: ModifierState,
}

/// One normalized keyboard shortcut used by feature-local input systems.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyShortcut {
    pub modifiers: ModifierMask,
    pub keycode: Option<u32>,
}

/// Activation style for one registered shortcut.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutTrigger {
    #[default]
    Press,
    Release,
    Hold,
}

/// One feature-owned shortcut registered into the global runtime registry.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub id: String,
    pub owner: String,
    pub description: String,
    pub default_binding: String,
    pub trigger: ShortcutTrigger,
}

/// Global feature-owned shortcut catalog.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutRegistry {
    shortcuts: BTreeMap<String, ShortcutSpec>,
}

/// One compiled runtime shortcut after defaults and config overrides have been resolved.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompiledShortcut {
    pub id: String,
    pub owner: String,
    pub description: String,
    pub binding: String,
    pub combo: KeyShortcut,
    pub trigger: ShortcutTrigger,
    pub overridden: bool,
}

/// Active compiled shortcut set used during input matching.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompiledShortcutMap {
    shortcuts: BTreeMap<String, CompiledShortcut>,
}

/// Per-shortcut activation state for the current frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutMatchState {
    pub active: bool,
    pub just_pressed: bool,
    pub just_released: bool,
}

/// Runtime activation state for all compiled shortcuts.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutState {
    matches: BTreeMap<String, ShortcutMatchState>,
}

/// Latest shortcut compile diagnostics exposed to IPC/debug consumers.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutCompileDiagnostics {
    pub last_error: Option<String>,
}

/// Typed modifier mask used for config-driven gestures that only care about held modifiers.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModifierMask {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl ModifierMask {
    /// Builds a typed modifier mask from explicit booleans.
    pub const fn new(ctrl: bool, alt: bool, shift: bool, logo: bool) -> Self {
        Self { ctrl, alt, shift, logo }
    }

    /// Returns whether the currently held modifiers satisfy this required mask.
    pub fn matches_required(&self, modifiers: &ModifierState) -> bool {
        (!self.ctrl || modifiers.ctrl)
            && (!self.alt || modifiers.alt)
            && (!self.shift || modifiers.shift)
            && (!self.logo || modifiers.logo)
    }

    /// Returns whether the mask requires no modifiers.
    pub fn is_empty(&self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.logo
    }

    /// Parses a modifier mask from config-style tokens such as `Ctrl` or `Super`.
    pub fn from_config_tokens<'a>(
        tokens: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, String> {
        let mut mask = Self::default();

        for token in tokens {
            match normalize_modifier_name(token) {
                Some("ctrl") => mask.ctrl = true,
                Some("alt") => mask.alt = true,
                Some("shift") => mask.shift = true,
                Some("logo") => mask.logo = true,
                Some(_) => unreachable!(),
                None => return Err(format!("unsupported modifier `{token}`")),
            }
        }

        if mask.is_empty() {
            return Err("viewport pan modifiers must include at least one modifier".to_owned());
        }

        Ok(mask)
    }

    /// Serializes the mask back into user-facing config tokens.
    pub fn config_tokens(&self) -> Vec<String> {
        let mut tokens = Vec::new();

        if self.logo {
            tokens.push("Super".to_owned());
        }
        if self.ctrl {
            tokens.push("Ctrl".to_owned());
        }
        if self.alt {
            tokens.push("Alt".to_owned());
        }
        if self.shift {
            tokens.push("Shift".to_owned());
        }

        tokens
    }
}

impl ShortcutSpec {
    /// Builds one registered shortcut spec owned by a specific feature.
    pub fn new(
        id: impl Into<String>,
        owner: impl Into<String>,
        description: impl Into<String>,
        default_binding: impl Into<String>,
        trigger: ShortcutTrigger,
    ) -> Self {
        Self {
            id: id.into(),
            owner: owner.into(),
            description: description.into(),
            default_binding: default_binding.into(),
            trigger,
        }
    }
}

impl ShortcutRegistry {
    /// Registers one feature-owned shortcut id into the global registry.
    pub fn register(&mut self, spec: ShortcutSpec) -> Result<(), String> {
        let id = spec.id.clone();
        if self.shortcuts.contains_key(&id) {
            return Err(format!("shortcut `{id}` was already registered"));
        }
        self.shortcuts.insert(id, spec);
        Ok(())
    }

    /// Returns the registered shortcut spec for one id.
    pub fn get(&self, id: &str) -> Option<&ShortcutSpec> {
        self.shortcuts.get(id)
    }

    /// Iterates all registered shortcuts in stable id order.
    pub fn iter(&self) -> impl Iterator<Item = &ShortcutSpec> {
        self.shortcuts.values()
    }

    /// Returns whether no shortcuts have been registered yet.
    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }
}

impl CompiledShortcutMap {
    /// Replaces the compiled shortcut set wholesale.
    pub fn replace(&mut self, shortcuts: BTreeMap<String, CompiledShortcut>) {
        self.shortcuts = shortcuts;
    }

    /// Returns the compiled shortcut for one id.
    pub fn get(&self, id: &str) -> Option<&CompiledShortcut> {
        self.shortcuts.get(id)
    }

    /// Iterates all compiled shortcuts in stable id order.
    pub fn iter(&self) -> impl Iterator<Item = &CompiledShortcut> {
        self.shortcuts.values()
    }

    /// Returns whether no compiled shortcuts are currently active.
    pub fn is_empty(&self) -> bool {
        self.shortcuts.is_empty()
    }
}

impl ShortcutState {
    /// Replaces the current shortcut activation state wholesale.
    pub fn replace(&mut self, matches: BTreeMap<String, ShortcutMatchState>) {
        self.matches = matches;
    }

    /// Inserts or overwrites the runtime state for one shortcut id.
    pub fn set(
        &mut self,
        id: impl Into<String>,
        active: bool,
        just_pressed: bool,
        just_released: bool,
    ) {
        self.matches.insert(id.into(), ShortcutMatchState { active, just_pressed, just_released });
    }

    /// Returns the current runtime state for one shortcut id.
    pub fn get(&self, id: &str) -> Option<&ShortcutMatchState> {
        self.matches.get(id)
    }

    /// Returns whether the shortcut is currently active.
    pub fn active(&self, id: &str) -> bool {
        self.get(id).is_some_and(|state| state.active)
    }

    /// Returns whether the shortcut was just pressed this frame.
    pub fn just_pressed(&self, id: &str) -> bool {
        self.get(id).is_some_and(|state| state.just_pressed)
    }

    /// Returns whether the shortcut was just released this frame.
    pub fn just_released(&self, id: &str) -> bool {
        self.get(id).is_some_and(|state| state.just_released)
    }
}

impl PressedKeys {
    /// Clears the per-frame transition sets while keeping held keys intact.
    pub fn clear_frame_transitions(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
    }

    /// Clears all held keys and modifier state.
    pub fn reset_all(&mut self) {
        self.held.clear();
        self.just_pressed.clear();
        self.just_released.clear();
        self.modifiers = ModifierState::default();
    }

    /// Records one key press or release and updates modifier state accordingly.
    pub fn record_key(&mut self, keycode: u32, pressed: bool) {
        if pressed {
            if self.held.insert(keycode) {
                self.just_pressed.insert(keycode);
            }
        } else if self.held.remove(&keycode) {
            self.just_released.insert(keycode);
        }

        update_modifier_state(&mut self.modifiers, keycode, pressed);
    }

    /// Returns the current modifier snapshot.
    pub fn modifiers(&self) -> &ModifierState {
        &self.modifiers
    }

    /// Returns the set of currently held keycodes.
    pub fn held(&self) -> &BTreeSet<u32> {
        &self.held
    }

    /// Returns whether the given key is currently held.
    pub fn is_key_held(&self, keycode: u32) -> bool {
        self.held.contains(&keycode)
    }

    /// Returns whether the key was first pressed during the current frame.
    pub fn was_key_just_pressed(&self, keycode: u32) -> bool {
        self.just_pressed.contains(&keycode)
    }

    /// Returns whether the key was released during the current frame.
    pub fn was_key_just_released(&self, keycode: u32) -> bool {
        self.just_released.contains(&keycode)
    }

    /// Returns whether a shortcut is currently active with an exact modifier match.
    pub fn is_pressed(&self, shortcut: &KeyShortcut) -> bool {
        shortcut.keycode.is_none_or(|keycode| self.held.contains(&keycode))
            && modifiers_match_exact(&self.modifiers, &shortcut.modifiers)
    }

    /// Returns whether a shortcut was just pressed this frame.
    pub fn just_pressed(&self, shortcut: &KeyShortcut) -> bool {
        shortcut.keycode.is_some_and(|keycode| self.just_pressed.contains(&keycode))
            && modifiers_match_exact(&self.modifiers, &shortcut.modifiers)
    }

    /// Returns whether a shortcut was just released this frame.
    pub fn just_released(&self, shortcut: &KeyShortcut) -> bool {
        shortcut.keycode.is_some_and(|keycode| self.just_released.contains(&keycode))
            && modifiers_match_exact(&self.modifiers, &shortcut.modifiers)
    }
}

impl KeyShortcut {
    /// Builds a typed shortcut from a modifier mask and optional keycode.
    pub const fn new(modifiers: ModifierMask, keycode: Option<u32>) -> Self {
        Self { modifiers, keycode }
    }

    /// Builds a modifier-only shortcut with no keycode.
    pub const fn modifier_only(modifiers: ModifierMask) -> Self {
        Self::new(modifiers, None)
    }

    /// Parses one config-facing combo string such as `Super+Shift+Q` or `Alt`.
    pub fn parse_config(binding: &str) -> Result<Self, String> {
        let mut modifiers = ModifierMask::default();
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
                        return Err(format!(
                            "shortcut `{binding}` must contain at most one non-modifier key"
                        ));
                    }
                    keycode = Some(
                        parse_non_modifier_keycode(token)
                            .ok_or_else(|| format!("unsupported key `{token}` in `{binding}`"))?,
                    );
                }
            }
        }

        if keycode.is_none() && modifiers.is_empty() {
            return Err("shortcut must contain at least one modifier or one key".to_owned());
        }

        Ok(Self::new(modifiers, keycode))
    }
}

fn modifiers_match_exact(current: &ModifierState, expected: &ModifierMask) -> bool {
    current.ctrl == expected.ctrl
        && current.alt == expected.alt
        && current.shift == expected.shift
        && current.logo == expected.logo
}

/// Updates the coarse modifier snapshot for one raw key transition.
pub fn update_modifier_state(modifiers: &mut ModifierState, keycode: u32, pressed: bool) {
    match keycode {
        37 | 105 => modifiers.ctrl = pressed,
        50 | 62 => modifiers.shift = pressed,
        64 | 108 => modifiers.alt = pressed,
        133 | 134 => modifiers.logo = pressed,
        _ => {}
    }
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

fn parse_non_modifier_keycode(token: &str) -> Option<u32> {
    match token.to_ascii_lowercase().as_str() {
        "1" => Some(10),
        "2" => Some(11),
        "3" => Some(12),
        "4" => Some(13),
        "5" => Some(14),
        "6" => Some(15),
        "7" => Some(16),
        "8" => Some(17),
        "9" => Some(18),
        "0" => Some(19),
        "q" => Some(24),
        "w" => Some(25),
        "e" => Some(26),
        "r" => Some(27),
        "t" => Some(28),
        "y" => Some(29),
        "u" => Some(30),
        "i" => Some(31),
        "o" => Some(32),
        "p" => Some(33),
        "a" => Some(38),
        "s" => Some(39),
        "d" => Some(40),
        "f" => Some(41),
        "g" => Some(42),
        "h" => Some(43),
        "j" => Some(44),
        "k" => Some(45),
        "l" => Some(46),
        "z" => Some(52),
        "x" => Some(53),
        "c" => Some(54),
        "v" => Some(55),
        "b" => Some(56),
        "n" => Some(57),
        "m" => Some(58),
        "tab" => Some(23),
        "return" | "enter" => Some(36),
        "space" => Some(65),
        "escape" | "esc" => Some(9),
        "backspace" => Some(22),
        "delete" => Some(119),
        "left" => Some(113),
        "right" => Some(114),
        "up" => Some(111),
        "down" => Some(116),
        "f1" => Some(67),
        "f2" => Some(68),
        "f3" => Some(69),
        "f4" => Some(70),
        "f5" => Some(71),
        "f6" => Some(72),
        "f7" => Some(73),
        "f8" => Some(74),
        "f9" => Some(75),
        "f10" => Some(76),
        "f11" => Some(95),
        "f12" => Some(96),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        CompiledShortcut, CompiledShortcutMap, KeyShortcut, ModifierMask, PressedKeys,
        ShortcutRegistry, ShortcutSpec, ShortcutState, ShortcutTrigger,
    };

    #[test]
    fn raw_key_queries_track_hold_and_frame_transitions() {
        let mut pressed = PressedKeys::default();
        pressed.record_key(23, true);

        assert!(pressed.is_key_held(23));
        assert!(pressed.was_key_just_pressed(23));
        assert!(!pressed.was_key_just_released(23));

        pressed.clear_frame_transitions();

        assert!(pressed.is_key_held(23));
        assert!(!pressed.was_key_just_pressed(23));

        pressed.record_key(23, false);

        assert!(!pressed.is_key_held(23));
        assert!(pressed.was_key_just_released(23));
    }

    #[test]
    fn required_modifier_match_allows_extra_modifiers() {
        let mut pressed = PressedKeys::default();
        pressed.record_key(64, true);
        pressed.record_key(50, true);

        assert!(ModifierMask::new(false, true, false, false).matches_required(pressed.modifiers()));
        assert!(ModifierMask::new(false, true, true, false).matches_required(pressed.modifiers()));
        assert!(!ModifierMask::new(true, true, false, false).matches_required(pressed.modifiers()));
    }

    #[test]
    fn key_shortcut_parse_supports_modifier_only_and_single_key_bindings() {
        let shortcut = KeyShortcut::parse_config("Super+Shift+Q").expect("parse full shortcut");
        assert_eq!(shortcut.modifiers, ModifierMask::new(false, false, true, true));
        assert_eq!(shortcut.keycode, Some(24));

        let modifier_only = KeyShortcut::parse_config("Alt").expect("parse modifier-only shortcut");
        assert_eq!(modifier_only.modifiers, ModifierMask::new(false, true, false, false));
        assert_eq!(modifier_only.keycode, None);
    }

    #[test]
    fn shortcut_registry_rejects_duplicate_ids() {
        let mut registry = ShortcutRegistry::default();
        registry
            .register(ShortcutSpec::new("system.quit", "system", "Quit", "Super+Shift+Q", ShortcutTrigger::Press))
            .expect("first registration");
        assert_eq!(
            registry.register(ShortcutSpec::new(
                "system.quit",
                "system",
                "Quit duplicate",
                "Alt+Q",
                ShortcutTrigger::Press
            )),
            Err("shortcut `system.quit` was already registered".to_owned())
        );
    }

    #[test]
    fn shortcut_state_queries_track_named_shortcuts() {
        let mut state = ShortcutState::default();
        state.set("window_switcher.hold", true, true, false);
        assert!(state.active("window_switcher.hold"));
        assert!(state.just_pressed("window_switcher.hold"));
        assert!(!state.just_released("window_switcher.hold"));
    }

    #[test]
    fn compiled_shortcut_map_replaces_wholesale() {
        let mut compiled = CompiledShortcutMap::default();
        compiled.replace(BTreeMap::from([(
            "system.quit".to_owned(),
            CompiledShortcut {
                id: "system.quit".to_owned(),
                owner: "system".to_owned(),
                description: "Quit".to_owned(),
                binding: "Super+Shift+Q".to_owned(),
                combo: KeyShortcut::parse_config("Super+Shift+Q").expect("parse"),
                trigger: ShortcutTrigger::Press,
                overridden: false,
            },
        )]));

        assert!(compiled.get("system.quit").is_some());
    }
}
