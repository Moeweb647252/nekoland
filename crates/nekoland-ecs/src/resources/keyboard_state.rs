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

/// Typed modifier mask used for config-driven gestures that only care about held modifiers.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModifierMask {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

impl ModifierMask {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, logo: bool) -> Self {
        Self { ctrl, alt, shift, logo }
    }

    pub fn matches_required(&self, modifiers: &ModifierState) -> bool {
        (!self.ctrl || modifiers.ctrl)
            && (!self.alt || modifiers.alt)
            && (!self.shift || modifiers.shift)
            && (!self.logo || modifiers.logo)
    }

    pub fn is_empty(&self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.logo
    }

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

fn normalize_modifier_name(token: &str) -> Option<&'static str> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some("ctrl"),
        "alt" => Some("alt"),
        "shift" => Some("shift"),
        "super" | "logo" | "meta" => Some("logo"),
        _ => None,
    }
}
