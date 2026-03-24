//! Keybinding schema wrappers used by the on-disk config format.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use crate::resources::ConfiguredAction;
use serde::{Deserialize, Serialize};

use crate::action_config::ActionListConfig;

/// One explicit keybinding entry as it may appear in docs, generated output, or future list-based
/// config formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeybindDefinition {
    pub combo: String,
    pub actions: Vec<ConfiguredAction>,
}

/// Current config-file representation of keybindings: a direct combo-to-action map.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeybindConfig {
    pub bindings: BTreeMap<String, ActionListConfig>,
}
