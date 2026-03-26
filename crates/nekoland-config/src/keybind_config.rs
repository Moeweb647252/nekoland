//! Shortcut override schema wrappers used by the on-disk config format.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One explicit keybinding entry as it may appear in docs, generated output, or future list-based
/// config formats.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeybindDefinition {
    pub shortcut_id: String,
    pub combo: String,
}

/// Current config-file representation of shortcut overrides: a direct shortcut-id to combo map.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeybindConfig {
    pub bindings: BTreeMap<String, String>,
}
