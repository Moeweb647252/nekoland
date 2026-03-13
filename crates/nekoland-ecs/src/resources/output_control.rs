use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::selectors::{OutputName, OutputSelector};

/// High-level output control updates staged by IPC, keybindings, or tests.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOutputControls {
    controls: Vec<PendingOutputControl>,
}

/// One staged control update for a single output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOutputControl {
    pub selector: OutputSelector,
    pub enabled: Option<bool>,
    pub configuration: Option<OutputControlConfiguration>,
}

/// Desired output configuration staged for backend reconciliation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputControlConfiguration {
    pub mode: String,
    pub scale: Option<u32>,
}

/// Mutable façade over one staged output control entry.
pub struct OutputControlHandle<'a> {
    control: &'a mut PendingOutputControl,
}

impl Default for PendingOutputControl {
    fn default() -> Self {
        Self { selector: OutputSelector::Primary, enabled: None, configuration: None }
    }
}

impl PendingOutputControls {
    pub fn select(&mut self, selector: OutputSelector) -> OutputControlHandle<'_> {
        let index = self.controls.iter().position(|control| control.selector == selector);
        let control = if let Some(index) = index {
            &mut self.controls[index]
        } else {
            self.controls
                .push(PendingOutputControl { selector, ..PendingOutputControl::default() });
            self.controls.last_mut().expect("output control just pushed")
        };

        OutputControlHandle { control }
    }

    pub fn named(&mut self, output: OutputName) -> OutputControlHandle<'_> {
        self.select(OutputSelector::Name(output))
    }

    pub fn primary(&mut self) -> OutputControlHandle<'_> {
        self.select(OutputSelector::Primary)
    }

    pub fn take(&mut self) -> Vec<PendingOutputControl> {
        std::mem::take(&mut self.controls)
    }

    pub fn replace(&mut self, controls: Vec<PendingOutputControl>) {
        self.controls = controls;
    }

    pub fn as_slice(&self) -> &[PendingOutputControl] {
        &self.controls
    }

    pub fn clear(&mut self) {
        self.controls.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

impl OutputControlHandle<'_> {
    pub fn enable(&mut self) -> &mut Self {
        self.control.enabled = Some(true);
        self
    }

    pub fn disable(&mut self) -> &mut Self {
        self.control.enabled = Some(false);
        self
    }

    pub fn configure(&mut self, mode: impl Into<String>, scale: Option<u32>) -> &mut Self {
        self.control.configuration = Some(OutputControlConfiguration { mode: mode.into(), scale });
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::selectors::{OutputName, OutputSelector};

    use super::{OutputControlConfiguration, PendingOutputControl, PendingOutputControls};

    #[test]
    fn named_controls_merge_enable_disable_and_configure() {
        let mut controls = PendingOutputControls::default();
        controls.named(OutputName::from("Virtual-1")).enable().configure("1600x900@75", Some(2));

        assert_eq!(
            controls.as_slice(),
            &[PendingOutputControl {
                selector: OutputSelector::Name(OutputName::from("Virtual-1")),
                enabled: Some(true),
                configuration: Some(OutputControlConfiguration {
                    mode: "1600x900@75".to_owned(),
                    scale: Some(2),
                }),
            }]
        );
    }
}
