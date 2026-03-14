use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::{Query, Res, ResMut};
use bevy_ecs::system::SystemParam;

use crate::components::{ActiveWorkspace, OutputDevice, WlSurfaceHandle, Workspace};
use crate::resources::{
    KeyboardFocusState, OutputControlHandle, PendingOutputControls, PendingWindowControls,
    PendingWorkspaceControls, WindowControlHandle,
};
use crate::selectors::{
    OutputName, OutputSelector, SurfaceId, WindowSelector, WorkspaceLookup, WorkspaceName,
    WorkspaceSelector,
};

/// Lightweight high-level API for staging window control updates from plain Rust helpers.
pub struct WindowControlApi<'a> {
    keyboard_focus: &'a KeyboardFocusState,
    pending: &'a mut PendingWindowControls,
}

impl<'a> WindowControlApi<'a> {
    pub fn new(
        keyboard_focus: &'a KeyboardFocusState,
        pending: &'a mut PendingWindowControls,
    ) -> Self {
        Self { keyboard_focus, pending }
    }

    pub fn select(&mut self, selector: WindowSelector) -> Option<WindowControlHandle<'_>> {
        self.pending.select(selector, self.keyboard_focus)
    }

    pub fn surface(&mut self, surface_id: SurfaceId) -> WindowControlHandle<'_> {
        self.pending.surface(surface_id)
    }

    pub fn focused(&mut self) -> Option<WindowControlHandle<'_>> {
        self.pending.focused(self.keyboard_focus)
    }

    pub fn focused_surface_id(&self) -> Option<SurfaceId> {
        self.keyboard_focus.focused_surface.map(SurfaceId)
    }
}

/// Lightweight high-level API for staging workspace control updates from plain Rust helpers.
pub struct WorkspaceControlApi<'a> {
    pending: &'a mut PendingWorkspaceControls,
}

impl<'a> WorkspaceControlApi<'a> {
    pub fn new(pending: &'a mut PendingWorkspaceControls) -> Self {
        Self { pending }
    }

    pub fn switch_or_create(&mut self, target: WorkspaceLookup) {
        self.pending.switch_or_create(target);
    }

    pub fn switch_or_create_named(&mut self, workspace: impl Into<WorkspaceName>) {
        self.pending.switch_or_create_named(workspace);
    }

    pub fn switch_or_create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.pending.switch_or_create_id(workspace);
    }

    pub fn create_named(&mut self, workspace: impl Into<WorkspaceName>) {
        self.pending.create_named(workspace);
    }

    pub fn create(&mut self, target: WorkspaceLookup) {
        self.pending.create(target);
    }

    pub fn create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.pending.create_id(workspace);
    }

    pub fn destroy(&mut self, target: WorkspaceSelector) {
        self.pending.destroy(target);
    }

    pub fn destroy_named(&mut self, workspace: impl Into<WorkspaceName>) {
        self.pending.destroy_named(workspace);
    }

    pub fn destroy_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.pending.destroy_id(workspace);
    }

    pub fn destroy_active(&mut self) {
        self.pending.destroy_active();
    }
}

/// Lightweight high-level API for staging output control updates from plain Rust helpers.
pub struct OutputControlApi<'a> {
    pending: &'a mut PendingOutputControls,
}

impl<'a> OutputControlApi<'a> {
    pub fn new(pending: &'a mut PendingOutputControls) -> Self {
        Self { pending }
    }

    pub fn select(&mut self, selector: OutputSelector) -> OutputControlHandle<'_> {
        self.pending.select(selector)
    }

    pub fn named(&mut self, output: OutputName) -> OutputControlHandle<'_> {
        self.pending.named(output)
    }

    pub fn primary(&mut self) -> OutputControlHandle<'_> {
        self.pending.primary()
    }

    pub fn focused(&mut self) -> OutputControlHandle<'_> {
        self.pending.select(OutputSelector::Focused)
    }
}

/// SystemParam façade over high-level window controls.
#[derive(SystemParam)]
pub struct WindowOps<'w, 's> {
    keyboard_focus: Res<'w, KeyboardFocusState>,
    pending: ResMut<'w, PendingWindowControls>,
    surfaces: Query<'w, 's, &'static WlSurfaceHandle>,
    _marker: std::marker::PhantomData<&'s ()>,
}

impl<'w, 's> WindowOps<'w, 's> {
    pub fn api(&mut self) -> WindowControlApi<'_> {
        WindowControlApi::new(&self.keyboard_focus, &mut self.pending)
    }

    pub fn select(&mut self, selector: WindowSelector) -> Option<WindowControlHandle<'_>> {
        self.pending.select(selector, &self.keyboard_focus)
    }

    pub fn surface(&mut self, surface_id: SurfaceId) -> WindowControlHandle<'_> {
        self.pending.surface(surface_id)
    }

    pub fn focused(&mut self) -> Option<WindowControlHandle<'_>> {
        self.pending.focused(&self.keyboard_focus)
    }

    pub fn focused_surface_id(&self) -> Option<SurfaceId> {
        self.keyboard_focus.focused_surface.map(SurfaceId)
    }

    pub fn entity(&mut self, entity: Entity) -> Option<WindowControlHandle<'_>> {
        self.surfaces.get(entity).ok().map(|surface| self.pending.surface(SurfaceId(surface.id)))
    }
}

/// SystemParam façade over high-level workspace controls.
#[derive(SystemParam)]
pub struct WorkspaceOps<'w, 's> {
    pending: ResMut<'w, PendingWorkspaceControls>,
    active_workspaces:
        Query<'w, 's, (Entity, &'static Workspace), bevy_ecs::query::With<ActiveWorkspace>>,
    _marker: std::marker::PhantomData<&'s ()>,
}

impl<'w, 's> WorkspaceOps<'w, 's> {
    pub fn api(&mut self) -> WorkspaceControlApi<'_> {
        WorkspaceControlApi::new(&mut self.pending)
    }

    pub fn switch_or_create(&mut self, target: WorkspaceLookup) {
        self.pending.switch_or_create(target);
    }

    pub fn switch_or_create_named(&mut self, workspace: impl Into<WorkspaceName>) {
        self.pending.switch_or_create_named(workspace);
    }

    pub fn switch_or_create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.pending.switch_or_create_id(workspace);
    }

    pub fn create_named(&mut self, workspace: impl Into<WorkspaceName>) {
        self.pending.create_named(workspace);
    }

    pub fn create(&mut self, target: WorkspaceLookup) {
        self.pending.create(target);
    }

    pub fn create_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.pending.create_id(workspace);
    }

    pub fn destroy(&mut self, target: WorkspaceSelector) {
        self.pending.destroy(target);
    }

    pub fn destroy_named(&mut self, workspace: impl Into<WorkspaceName>) {
        self.pending.destroy_named(workspace);
    }

    pub fn destroy_id(&mut self, workspace: crate::components::WorkspaceId) {
        self.pending.destroy_id(workspace);
    }

    pub fn destroy_active(&mut self) {
        self.pending.destroy_active();
    }

    pub fn active_entity(&self) -> Option<Entity> {
        self.active_workspaces.iter().next().map(|(entity, _)| entity)
    }

    pub fn active_workspace(&self) -> Option<&Workspace> {
        self.active_workspaces.iter().next().map(|(_, workspace)| workspace)
    }
}

/// SystemParam façade over high-level output controls.
#[derive(SystemParam)]
pub struct OutputOps<'w, 's> {
    pending: ResMut<'w, PendingOutputControls>,
    outputs: Query<'w, 's, (Entity, &'static OutputDevice)>,
    _marker: std::marker::PhantomData<&'s ()>,
}

impl<'w, 's> OutputOps<'w, 's> {
    pub fn api(&mut self) -> OutputControlApi<'_> {
        OutputControlApi::new(&mut self.pending)
    }

    pub fn select(&mut self, selector: OutputSelector) -> OutputControlHandle<'_> {
        self.pending.select(selector)
    }

    pub fn named(&mut self, output: OutputName) -> OutputControlHandle<'_> {
        self.pending.named(output)
    }

    pub fn primary(&mut self) -> OutputControlHandle<'_> {
        self.pending.primary()
    }

    pub fn focused(&mut self) -> OutputControlHandle<'_> {
        self.pending.select(OutputSelector::Focused)
    }

    pub fn entity_named(&self, output: &OutputName) -> Option<Entity> {
        self.outputs
            .iter()
            .find(|(_, device)| device.name == output.as_str())
            .map(|(entity, _)| entity)
    }
}

#[cfg(test)]
mod tests {
    use crate::resources::{
        KeyboardFocusState, PendingOutputControls, PendingWindowControls, PendingWorkspaceControls,
    };
    use crate::selectors::{OutputName, SurfaceId, WorkspaceName};

    use super::{OutputControlApi, WindowControlApi, WorkspaceControlApi};

    #[test]
    fn plain_control_apis_stage_updates_without_system_param() {
        let focus = KeyboardFocusState { focused_surface: Some(7) };
        let mut windows = PendingWindowControls::default();
        let mut workspaces = PendingWorkspaceControls::default();
        let mut outputs = PendingOutputControls::default();

        WindowControlApi::new(&focus, &mut windows).surface(SurfaceId(7)).close();
        WorkspaceControlApi::new(&mut workspaces).switch_or_create_named(WorkspaceName::from("2"));
        OutputControlApi::new(&mut outputs).named(OutputName::from("Virtual-1")).enable();

        assert!(windows.as_slice()[0].close);
        assert_eq!(workspaces.as_slice().len(), 1);
        assert_eq!(outputs.as_slice()[0].enabled, Some(true));
    }
}
