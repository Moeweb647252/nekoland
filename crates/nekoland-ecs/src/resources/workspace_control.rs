//! High-level workspace control queues used by IPC, keybindings, and shell policy.

#![allow(missing_docs)]

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::selectors::{WorkspaceLookup, WorkspaceSelector};

/// High-level workspace control operations staged by IPC, keybindings, or other shell systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWorkspaceControls {
    controls: Vec<WorkspaceControl>,
}

/// One staged workspace control operation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceControl {
    SwitchOrCreate { target: WorkspaceLookup },
    Create { target: WorkspaceLookup },
    Destroy { target: WorkspaceSelector },
}

impl PendingWorkspaceControls {
    /// Stages a switch-or-create request for the given workspace lookup.
    pub fn switch_or_create(&mut self, target: WorkspaceLookup) {
        self.controls.push(WorkspaceControl::SwitchOrCreate { target });
    }

    /// Stages a switch-or-create request by workspace name.
    pub fn switch_or_create_named(
        &mut self,
        workspace: impl Into<crate::selectors::WorkspaceName>,
    ) {
        self.switch_or_create(WorkspaceLookup::Name(workspace.into()));
    }

    /// Stages a switch-or-create request by workspace id.
    pub fn switch_or_create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.switch_or_create(WorkspaceLookup::Id(workspace));
    }

    /// Stages an explicit create request.
    pub fn create(&mut self, target: WorkspaceLookup) {
        self.controls.push(WorkspaceControl::Create { target });
    }

    /// Stages an explicit create request by workspace name.
    pub fn create_named(&mut self, workspace: impl Into<crate::selectors::WorkspaceName>) {
        self.create(WorkspaceLookup::Name(workspace.into()));
    }

    /// Stages an explicit create request by workspace id.
    pub fn create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.create(WorkspaceLookup::Id(workspace));
    }

    /// Stages a destroy request for the given selector.
    pub fn destroy(&mut self, target: WorkspaceSelector) {
        self.controls.push(WorkspaceControl::Destroy { target });
    }

    /// Stages a destroy request by workspace name.
    pub fn destroy_named(&mut self, workspace: impl Into<crate::selectors::WorkspaceName>) {
        self.destroy(WorkspaceSelector::Name(workspace.into()));
    }

    /// Stages a destroy request by workspace id.
    pub fn destroy_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.destroy(WorkspaceSelector::Id(workspace));
    }

    /// Stages a destroy request targeting the active workspace.
    pub fn destroy_active(&mut self) {
        self.destroy(WorkspaceSelector::Active);
    }

    /// Drains all staged workspace controls for the frame.
    pub fn take(&mut self) -> Vec<WorkspaceControl> {
        std::mem::take(&mut self.controls)
    }

    /// Replaces the staged workspace control list.
    pub fn replace(&mut self, controls: Vec<WorkspaceControl>) {
        self.controls = controls;
    }

    /// Returns the staged workspace controls as a slice.
    pub fn as_slice(&self) -> &[WorkspaceControl] {
        &self.controls
    }

    /// Clears all staged workspace controls.
    pub fn clear(&mut self) {
        self.controls.clear();
    }

    /// Returns whether no workspace controls are currently staged.
    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::components::WorkspaceId;
    use crate::selectors::{WorkspaceLookup, WorkspaceName, WorkspaceSelector};

    use super::{PendingWorkspaceControls, WorkspaceControl};

    #[test]
    fn stages_workspace_controls_in_order() {
        let mut controls = PendingWorkspaceControls::default();
        controls.create_id(WorkspaceId(2));
        controls.switch_or_create_named("dev");
        controls.destroy_named("1");

        assert_eq!(
            controls.as_slice(),
            &[
                WorkspaceControl::Create { target: WorkspaceLookup::Id(WorkspaceId(2)) },
                WorkspaceControl::SwitchOrCreate {
                    target: WorkspaceLookup::Name(WorkspaceName::from("dev"))
                },
                WorkspaceControl::Destroy {
                    target: WorkspaceSelector::Name(WorkspaceName::from("1"))
                },
            ]
        );
    }

    #[test]
    fn destroy_active_stages_explicit_active_selector() {
        let mut controls = PendingWorkspaceControls::default();
        controls.destroy_active();

        assert_eq!(
            controls.as_slice(),
            &[WorkspaceControl::Destroy { target: WorkspaceSelector::Active }]
        );
    }
}
