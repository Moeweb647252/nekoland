use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::resources::ConfiguredKeyboardLayout;

/// Default seat name used by compositor-owned keyboard-layout state snapshots.
pub const DEFAULT_KEYBOARD_SEAT_NAME: &str = "seat-0";

/// Runtime keyboard-layout state for the compositor seat.
#[derive(Resource, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardLayoutState {
    seat_name: String,
    layouts: Vec<ConfiguredKeyboardLayout>,
    active_index: usize,
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
    /// Returns the logical seat name associated with this layout state.
    pub fn seat_name(&self) -> &str {
        &self.seat_name
    }

    /// Returns the normalized list of configured keyboard layouts.
    pub fn layouts(&self) -> &[ConfiguredKeyboardLayout] {
        &self.layouts
    }

    /// Returns the currently active layout index after bounds normalization.
    pub fn active_index(&self) -> usize {
        self.normalized_active_index()
    }

    /// Builds runtime keyboard-layout state from normalized config data.
    pub fn from_config(layouts: &[ConfiguredKeyboardLayout], current_name: &str) -> Self {
        let mut state = Self::default();
        state.apply_layouts(layouts, Some(current_name), None);
        state
    }

    /// Returns the active keyboard-layout entry.
    pub fn active_layout(&self) -> &ConfiguredKeyboardLayout {
        debug_assert!(
            !self.layouts.is_empty(),
            "keyboard layout state should always contain at least one layout"
        );
        &self.layouts[self.normalized_active_index()]
    }

    /// Returns the active keyboard-layout name.
    pub fn active_name(&self) -> &str {
        self.active_layout().name.as_str()
    }

    /// Replaces the configured layout list while preserving the preferred active layout when possible.
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

    /// Activates the next configured layout, wrapping around at the end of the list.
    pub fn activate_next(&mut self) -> bool {
        if self.layouts.is_empty() {
            return false;
        }

        let next_index = (self.normalized_active_index() + 1) % self.layouts.len();
        self.activate_index(next_index)
    }

    /// Activates the previous configured layout, wrapping around at the start of the list.
    pub fn activate_prev(&mut self) -> bool {
        if self.layouts.is_empty() {
            return false;
        }

        let current_index = self.normalized_active_index();
        let prev_index =
            if current_index == 0 { self.layouts.len() - 1 } else { current_index - 1 };
        self.activate_index(prev_index)
    }

    /// Activates the layout at the provided index when it exists and is not already active.
    pub fn activate_index(&mut self, index: usize) -> bool {
        if index >= self.layouts.len() || self.active_index == index {
            return false;
        }

        self.active_index = index;
        true
    }

    /// Activates the layout with the provided name when present.
    pub fn activate_name(&mut self, name: &str) -> bool {
        let Some(index) = self.index_for_name(Some(name)) else {
            return false;
        };
        self.activate_index(index)
    }

    /// Returns whether a layout with the provided name exists.
    pub fn contains_name(&self, name: &str) -> bool {
        self.index_for_name(Some(name)).is_some()
    }

    fn index_for_name(&self, name: Option<&str>) -> Option<usize> {
        let name = name?;
        self.layouts.iter().position(|layout| layout.name == name)
    }

    fn normalized_active_index(&self) -> usize {
        self.active_index.min(self.layouts.len().saturating_sub(1))
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

    #[test]
    fn active_layout_clamps_stale_index_back_to_the_last_available_layout() {
        let mut state = KeyboardLayoutState::from_config(&[layout("us", "us")], "us");
        state.active_index = usize::MAX;

        assert_eq!(state.active_index(), 0);
        assert_eq!(state.active_name(), "us");
    }
}
