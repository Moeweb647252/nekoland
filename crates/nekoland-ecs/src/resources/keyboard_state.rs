//! Keyboard focus, modifier, and pressed-key snapshots used by input-driven systems.

#![allow(missing_docs)]

use std::collections::BTreeSet;

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

#[cfg(test)]
mod tests {
    use super::{ModifierMask, PressedKeys};

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
}
