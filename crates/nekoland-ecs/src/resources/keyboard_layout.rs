use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::ConfiguredKeyboardLayout;

pub const DEFAULT_KEYBOARD_SEAT_NAME: &str = "seat-0";

/// Runtime keyboard-layout state for the compositor seat.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardLayoutState {
    pub seat_name: String,
    pub layouts: Vec<ConfiguredKeyboardLayout>,
    pub active_index: usize,
}

impl Default for KeyboardLayoutState {
    fn default() -> Self {
        Self {
            seat_name: DEFAULT_KEYBOARD_SEAT_NAME.to_owned(),
            layouts: vec![ConfiguredKeyboardLayout::default()],
            active_index: 0,
        }
    }
}

impl KeyboardLayoutState {
    pub fn from_config(layouts: &[ConfiguredKeyboardLayout], current_name: &str) -> Self {
        let mut state = Self::default();
        state.apply_layouts(layouts, Some(current_name), None);
        state
    }

    pub fn active_layout(&self) -> &ConfiguredKeyboardLayout {
        self.layouts
            .get(self.active_index)
            .or_else(|| self.layouts.first())
            .expect("keyboard layout state should always contain at least one layout")
    }

    pub fn active_name(&self) -> &str {
        self.active_layout().name.as_str()
    }

    pub fn apply_layouts(
        &mut self,
        layouts: &[ConfiguredKeyboardLayout],
        configured_current: Option<&str>,
        preferred_current: Option<&str>,
    ) {
        self.layouts = if layouts.is_empty() {
            vec![ConfiguredKeyboardLayout::default()]
        } else {
            layouts.to_vec()
        };
        self.active_index = self
            .index_for_name(preferred_current)
            .or_else(|| self.index_for_name(configured_current))
            .unwrap_or(0);
    }

    pub fn activate_next(&mut self) -> bool {
        if self.layouts.is_empty() {
            return false;
        }

        let next_index = (self.active_index + 1) % self.layouts.len();
        self.activate_index(next_index)
    }

    pub fn activate_prev(&mut self) -> bool {
        if self.layouts.is_empty() {
            return false;
        }

        let prev_index = if self.active_index == 0 {
            self.layouts.len() - 1
        } else {
            self.active_index - 1
        };
        self.activate_index(prev_index)
    }

    pub fn activate_index(&mut self, index: usize) -> bool {
        if index >= self.layouts.len() || self.active_index == index {
            return false;
        }

        self.active_index = index;
        true
    }

    pub fn activate_name(&mut self, name: &str) -> bool {
        let Some(index) = self.index_for_name(Some(name)) else {
            return false;
        };
        self.activate_index(index)
    }

    fn index_for_name(&self, name: Option<&str>) -> Option<usize> {
        let name = name?;
        self.layouts.iter().position(|layout| layout.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::KeyboardLayoutState;
    use crate::resources::ConfiguredKeyboardLayout;

    fn layout(name: &str, layout: &str) -> ConfiguredKeyboardLayout {
        ConfiguredKeyboardLayout {
            name: name.to_owned(),
            layout: layout.to_owned(),
            ..ConfiguredKeyboardLayout::default()
        }
    }

    #[test]
    fn apply_layouts_prefers_previous_active_name_over_config_default() {
        let mut state =
            KeyboardLayoutState::from_config(&[layout("us", "us"), layout("de", "de")], "us");
        assert!(state.activate_name("de"));

        state.apply_layouts(
            &[layout("us", "us"), layout("de", "de"), layout("fr", "fr")],
            Some("us"),
            Some("de"),
        );

        assert_eq!(state.active_name(), "de");
    }

    #[test]
    fn apply_layouts_falls_back_to_config_default_when_previous_name_disappears() {
        let mut state =
            KeyboardLayoutState::from_config(&[layout("us", "us"), layout("de", "de")], "us");
        assert!(state.activate_name("de"));

        state.apply_layouts(&[layout("us", "us"), layout("fr", "fr")], Some("fr"), Some("de"));

        assert_eq!(state.active_name(), "fr");
    }
}
