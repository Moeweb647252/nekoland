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
    pub fn switch_or_create(&mut self, target: WorkspaceLookup) {
        self.controls.push(WorkspaceControl::SwitchOrCreate { target });
    }

    pub fn switch_or_create_named(
        &mut self,
        workspace: impl Into<crate::selectors::WorkspaceName>,
    ) {
        self.switch_or_create(WorkspaceLookup::Name(workspace.into()));
    }

    pub fn switch_or_create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.switch_or_create(WorkspaceLookup::Id(workspace));
    }

    pub fn create(&mut self, target: WorkspaceLookup) {
        self.controls.push(WorkspaceControl::Create { target });
    }

    pub fn create_named(&mut self, workspace: impl Into<crate::selectors::WorkspaceName>) {
        self.create(WorkspaceLookup::Name(workspace.into()));
    }

    pub fn create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.create(WorkspaceLookup::Id(workspace));
    }

    pub fn destroy(&mut self, target: WorkspaceSelector) {
        self.controls.push(WorkspaceControl::Destroy { target });
    }

    pub fn destroy_named(&mut self, workspace: impl Into<crate::selectors::WorkspaceName>) {
        self.destroy(WorkspaceSelector::Name(workspace.into()));
    }

    pub fn destroy_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.destroy(WorkspaceSelector::Id(workspace));
    }

    pub fn destroy_active(&mut self) {
        self.destroy(WorkspaceSelector::Active);
    }

    pub fn take(&mut self) -> Vec<WorkspaceControl> {
        std::mem::take(&mut self.controls)
    }

    pub fn replace(&mut self, controls: Vec<WorkspaceControl>) {
        self.controls = controls;
    }

    pub fn as_slice(&self) -> &[WorkspaceControl] {
        &self.controls
    }

    pub fn clear(&mut self) {
        self.controls.clear();
    }

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
