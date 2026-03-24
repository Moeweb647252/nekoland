//! Config-facing action encodings used before normalization into runtime control requests.
//!
//! This module is intentionally schema-heavy: many public variants and fields map one-to-one to
//! the on-disk config format, so type-level documentation carries most of the meaning.
#![allow(missing_docs)]

use crate::resources::ConfiguredAction;
use nekoland_ecs::selectors::{OutputName, WorkspaceLookup, WorkspaceSelector};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeybindEntryConfig {
    Actions(Vec<ConfiguredAction>),
    ViewportPanMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ActionListConfig {
    One(ConfiguredActionConfig),
    Many(Vec<ConfiguredActionConfig>),
}

impl ActionListConfig {
    pub fn into_actions(self) -> Result<Vec<ConfiguredAction>, String> {
        match self {
            Self::One(action) => Ok(vec![action.try_into()?]),
            Self::Many(actions) => actions.into_iter().map(TryInto::try_into).collect(),
        }
    }

    pub fn into_keybind_entry(self) -> Result<KeybindEntryConfig, String> {
        match self {
            Self::One(ConfiguredActionConfig::ViewportPanMode { viewport_pan_mode }) => {
                require_flag("viewport_pan_mode", viewport_pan_mode)
                    .map(|()| KeybindEntryConfig::ViewportPanMode)
            }
            Self::One(action) => Ok(KeybindEntryConfig::Actions(vec![action.try_into()?])),
            Self::Many(actions) => {
                if actions
                    .iter()
                    .any(|action| matches!(action, ConfiguredActionConfig::ViewportPanMode { .. }))
                {
                    return Err(
                        "`viewport_pan_mode` must be the only action in a binding".to_owned()
                    );
                }
                actions
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()
                    .map(KeybindEntryConfig::Actions)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ConfiguredActionConfig {
    Exec { exec: Vec<String> },
    Close { close: bool },
    Move { r#move: [isize; 2] },
    Resize { resize: [u32; 2] },
    Split { split: nekoland_ecs::resources::SplitAxis },
    Background { background: OutputName },
    ClearBackground { clear_background: bool },
    Workspace { workspace: WorkspaceLookup },
    WorkspaceCreate { workspace_create: WorkspaceLookup },
    WorkspaceDestroy { workspace_destroy: WorkspaceDestroyTargetConfig },
    OutputEnable { output_enable: OutputName },
    OutputDisable { output_disable: OutputName },
    OutputConfigure { output_configure: OutputConfigureConfig },
    ViewportPan { viewport_pan: [isize; 2] },
    ViewportMove { viewport_move: [isize; 2] },
    ViewportCenter { viewport_center: bool },
    ViewportPanMode { viewport_pan_mode: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputConfigureConfig {
    pub output: OutputName,
    pub mode: String,
    #[serde(default)]
    pub scale: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum WorkspaceDestroyTargetConfig {
    Id(u32),
    Name(String),
}

impl TryFrom<ConfiguredActionConfig> for ConfiguredAction {
    type Error = String;

    fn try_from(value: ConfiguredActionConfig) -> Result<Self, Self::Error> {
        match value {
            ConfiguredActionConfig::Exec { exec } => {
                let Some(program) = exec.first() else {
                    return Err("`exec` must include at least one argv element".to_owned());
                };
                if program.trim().is_empty() {
                    return Err("`exec` must not start with an empty program".to_owned());
                }
                Ok(Self::Exec { argv: exec })
            }
            ConfiguredActionConfig::Close { close } => {
                require_flag("close", close).map(|()| Self::CloseFocusedWindow)
            }
            ConfiguredActionConfig::Move { r#move } => {
                Ok(Self::MoveFocusedWindow { x: r#move[0], y: r#move[1] })
            }
            ConfiguredActionConfig::Resize { resize } => {
                Ok(Self::ResizeFocusedWindow { width: resize[0], height: resize[1] })
            }
            ConfiguredActionConfig::Split { split } => Ok(Self::SplitFocusedWindow { axis: split }),
            ConfiguredActionConfig::Background { background } => {
                Ok(Self::BackgroundFocusedWindow { output: background })
            }
            ConfiguredActionConfig::ClearBackground { clear_background } => {
                require_flag("clear_background", clear_background)
                    .map(|()| Self::ClearFocusedWindowBackground)
            }
            ConfiguredActionConfig::Workspace { workspace } => {
                Ok(Self::SwitchWorkspace { workspace })
            }
            ConfiguredActionConfig::WorkspaceCreate { workspace_create } => {
                Ok(Self::CreateWorkspace { workspace: workspace_create })
            }
            ConfiguredActionConfig::WorkspaceDestroy { workspace_destroy } => {
                Ok(Self::DestroyWorkspace {
                    workspace: match workspace_destroy {
                        WorkspaceDestroyTargetConfig::Id(id) => {
                            WorkspaceSelector::Id(nekoland_ecs::components::WorkspaceId(id))
                        }
                        WorkspaceDestroyTargetConfig::Name(name)
                            if name.eq_ignore_ascii_case("active") =>
                        {
                            WorkspaceSelector::Active
                        }
                        WorkspaceDestroyTargetConfig::Name(name) => {
                            WorkspaceSelector::Name(name.into())
                        }
                    },
                })
            }
            ConfiguredActionConfig::OutputEnable { output_enable } => {
                Ok(Self::EnableOutput { output: output_enable })
            }
            ConfiguredActionConfig::OutputDisable { output_disable } => {
                Ok(Self::DisableOutput { output: output_disable })
            }
            ConfiguredActionConfig::OutputConfigure { output_configure } => {
                Ok(Self::ConfigureOutput {
                    output: output_configure.output,
                    mode: output_configure.mode,
                    scale: output_configure.scale,
                })
            }
            ConfiguredActionConfig::ViewportPan { viewport_pan } => {
                Ok(Self::PanViewport { delta_x: viewport_pan[0], delta_y: viewport_pan[1] })
            }
            ConfiguredActionConfig::ViewportMove { viewport_move } => {
                Ok(Self::MoveViewport { x: viewport_move[0], y: viewport_move[1] })
            }
            ConfiguredActionConfig::ViewportCenter { viewport_center } => {
                require_flag("viewport_center", viewport_center)
                    .map(|()| Self::CenterViewportOnFocusedWindow)
            }
            ConfiguredActionConfig::ViewportPanMode { .. } => {
                Err("`viewport_pan_mode` is only supported inside `[keybinds.bindings]`".to_owned())
            }
        }
    }
}

fn require_flag(name: &str, value: bool) -> Result<(), String> {
    if value { Ok(()) } else { Err(format!("`{name}` must be true when present")) }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use crate::resources::ConfiguredAction;
    use nekoland_ecs::selectors::WorkspaceSelector;

    use super::{ActionListConfig, ConfiguredActionConfig, KeybindEntryConfig};

    #[derive(Debug, Deserialize)]
    struct OneAction {
        action: ConfiguredActionConfig,
    }

    #[derive(Debug, Deserialize)]
    struct ManyActions {
        actions: ActionListConfig,
    }

    #[test]
    fn one_or_many_short_actions_normalize_into_runtime_actions() {
        let Ok(one) = toml::from_str::<OneAction>("action = { exec = [\"foot\"] }") else {
            panic!("single action should parse");
        };
        let one = one.action;
        let Ok(many) =
            toml::from_str::<ManyActions>("actions = [{ workspace = 1 }, { exec = [\"foot\"] }]")
        else {
            panic!("action list should parse");
        };
        let many = many.actions;

        let Ok(one_actions) = ActionListConfig::One(one).into_actions() else {
            panic!("single action should normalize");
        };
        assert_eq!(one_actions, vec![ConfiguredAction::Exec { argv: vec!["foot".to_owned()] }]);
        let Ok(many_actions) = many.into_actions() else {
            panic!("action list should normalize");
        };
        assert_eq!(
            many_actions,
            vec![
                ConfiguredAction::SwitchWorkspace {
                    workspace: nekoland_ecs::selectors::WorkspaceLookup::Id(
                        nekoland_ecs::components::WorkspaceId(1),
                    ),
                },
                ConfiguredAction::Exec { argv: vec!["foot".to_owned()] },
            ]
        );
    }

    #[test]
    fn workspace_destroy_accepts_active_keyword() {
        let Ok(action) = toml::from_str::<OneAction>("action = { workspace_destroy = \"active\" }")
        else {
            panic!("destroy action should parse");
        };
        let action = action.action;

        let Ok(action) = ConfiguredAction::try_from(action) else {
            panic!("destroy action should normalize");
        };
        assert_eq!(
            action,
            ConfiguredAction::DestroyWorkspace { workspace: WorkspaceSelector::Active }
        );
    }

    #[test]
    fn viewport_pan_mode_is_keybind_only() {
        let Ok(action) = toml::from_str::<OneAction>("action = { viewport_pan_mode = true }")
        else {
            panic!("viewport pan mode action should parse");
        };
        let action = action.action;

        assert_eq!(
            ConfiguredAction::try_from(action.clone()),
            Err("`viewport_pan_mode` is only supported inside `[keybinds.bindings]`".to_owned())
        );
        let Ok(keybind_entry) = ActionListConfig::One(action).into_keybind_entry() else {
            panic!("keybind entry should parse");
        };
        assert_eq!(keybind_entry, KeybindEntryConfig::ViewportPanMode);
    }

    #[test]
    fn viewport_pan_mode_must_be_alone_in_binding() {
        let Ok(actions) = toml::from_str::<ManyActions>(
            "actions = [{ viewport_pan_mode = true }, { exec = [\"foot\"] }]",
        ) else {
            panic!("action list should parse");
        };
        let actions = actions.actions;

        assert_eq!(
            actions.into_keybind_entry(),
            Err("`viewport_pan_mode` must be the only action in a binding".to_owned())
        );
    }
}
