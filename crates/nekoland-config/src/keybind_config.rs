use std::collections::BTreeMap;

use nekoland_ecs::resources::ConfiguredKeybindingAction;
use serde::{Deserialize, Serialize};

/// One explicit keybinding entry as it may appear in docs, generated output, or future list-based
/// config formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeybindDefinition {
    pub combo: String,
    pub action: ConfiguredKeybindingAction,
}

/// Current config-file representation of keybindings: a direct combo-to-action map.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeybindConfig {
    pub bindings: BTreeMap<String, ConfiguredKeybindingAction>,
}
