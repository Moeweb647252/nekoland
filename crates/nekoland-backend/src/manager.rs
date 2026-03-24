use bevy_app::App;
use bevy_ecs::prelude::Resource;
use nekoland_ecs::resources::{
    PlatformBackendDescriptor, PlatformBackendKind, PlatformBackendRole, PlatformBackendState,
};
use nekoland_protocol::ProtocolDmabufSupport;
use serde::{Deserialize, Serialize};
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use crate::drm::DrmRuntime;
use crate::traits::{
    Backend, BackendApplyCtx, BackendCapabilities, BackendDescriptor, BackendExtractCtx, BackendId,
    BackendKind, BackendPresentCtx, BackendRole,
};
use crate::virtual_output::VirtualRuntime;
use crate::winit::backend::WinitRuntime;

thread_local! {
    static REQUESTED_BACKEND_OVERRIDE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// ECS-facing snapshot of the currently active backend runtimes.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendStatus {
    /// Descriptors for every runtime backend currently installed in the app.
    pub active: Vec<BackendDescriptor>,
}

impl BackendStatus {
    /// Return the first backend acting as a display-producing runtime.
    pub fn primary_display(&self) -> Option<&BackendDescriptor> {
        self.active.iter().find(|descriptor| {
            descriptor.role == BackendRole::PrimaryDisplay
                || descriptor.role == BackendRole::SecondaryDisplay
        })
    }

    pub fn platform_state(&self) -> PlatformBackendState {
        PlatformBackendState {
            active: self
                .active
                .iter()
                .map(|descriptor| PlatformBackendDescriptor {
                    id: descriptor.id.0,
                    kind: match descriptor.kind {
                        BackendKind::Drm => PlatformBackendKind::Drm,
                        BackendKind::Winit => PlatformBackendKind::Winit,
                        BackendKind::Virtual => PlatformBackendKind::Virtual,
                        BackendKind::Auto => PlatformBackendKind::Auto,
                    },
                    role: match descriptor.role {
                        BackendRole::PrimaryDisplay => PlatformBackendRole::PrimaryDisplay,
                        BackendRole::SecondaryDisplay => PlatformBackendRole::SecondaryDisplay,
                        BackendRole::CaptureSink => PlatformBackendRole::CaptureSink,
                        BackendRole::DebugSink => PlatformBackendRole::DebugSink,
                    },
                    label: descriptor.label.clone(),
                    description: descriptor.description.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeededBackendOutput {
    /// Backend that can materialize the requested output blueprint.
    pub backend_id: BackendId,
    /// Output template returned by the backend for later ECS insertion.
    pub blueprint: crate::common::outputs::BackendOutputBlueprint,
}

/// Owns all active backend runtime instances.
pub struct BackendManager {
    /// Monotonic id source for newly installed backend runtimes.
    next_backend_id: u64,
    /// Heterogeneous list of installed runtime backends.
    backends: Vec<Box<dyn Backend>>,
}

#[derive(Clone, Default)]
pub struct SharedBackendManager(Rc<RefCell<BackendManager>>);

impl SharedBackendManager {
    pub fn new(manager: BackendManager) -> Self {
        Self(Rc::new(RefCell::new(manager)))
    }

    pub fn borrow(&self) -> Ref<'_, BackendManager> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, BackendManager> {
        self.0.borrow_mut()
    }
}

impl Default for BackendManager {
    fn default() -> Self {
        Self { next_backend_id: 1, backends: Vec::new() }
    }
}

impl BackendManager {
    /// Allocate one fresh runtime backend id.
    fn allocate_id(&mut self) -> BackendId {
        let id = BackendId(self.next_backend_id);
        self.next_backend_id = self.next_backend_id.saturating_add(1);
        id
    }

    /// Install every requested backend runtime and fall back to `winit` when
    /// the request list ends up empty.
    pub fn bootstrap(app: &mut App) -> Self {
        let mut manager = Self::default();
        for kind in requested_backend_kinds() {
            let id = manager.allocate_id();
            match kind {
                BackendKind::Drm => manager.backends.push(Box::new(DrmRuntime::install(app, id))),
                BackendKind::Winit => {
                    manager.backends.push(Box::new(WinitRuntime::install(app, id)))
                }
                BackendKind::Virtual => {
                    manager.backends.push(Box::new(VirtualRuntime::install(app, id)))
                }
                BackendKind::Auto => {}
            }
        }

        if manager.backends.is_empty() {
            let id = manager.allocate_id();
            manager.backends.push(Box::new(WinitRuntime::install(app, id)));
        }

        manager
    }

    /// Snapshot the currently installed backends into an ECS-friendly resource.
    pub fn snapshot(&self) -> BackendStatus {
        BackendStatus { active: self.backends.iter().map(|backend| backend.descriptor()).collect() }
    }

    /// Ask installed backends whether any of them can seed the named output.
    pub fn seed_output(&self, output_name: &str) -> Option<SeededBackendOutput> {
        for backend in &self.backends {
            let capabilities = backend.capabilities();
            if !capabilities.contains(
                BackendCapabilities::OUTPUT_DISCOVERY | BackendCapabilities::OUTPUT_CONFIGURATION,
            ) && !capabilities.contains(BackendCapabilities::OUTPUT_DISCOVERY)
            {
                continue;
            }

            if let Some(blueprint) = backend.seed_output(output_name) {
                return Some(SeededBackendOutput { backend_id: backend.id(), blueprint });
            }
        }
        None
    }

    /// Run the extract phase for every installed backend runtime.
    pub fn extract_all(
        &mut self,
        cx: &mut BackendExtractCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        for backend in &mut self.backends {
            backend.extract(cx)?;
        }
        Ok(())
    }

    /// Run the apply phase for every installed backend runtime.
    pub fn apply_all(
        &mut self,
        cx: &mut BackendApplyCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        for backend in &mut self.backends {
            backend.apply(cx)?;
        }
        Ok(())
    }

    /// Run the present phase for every installed backend runtime.
    pub fn present_all(
        &mut self,
        cx: &mut BackendPresentCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        for backend in &mut self.backends {
            backend.present(cx)?;
        }
        Ok(())
    }

    pub fn collect_protocol_dmabuf_support(
        &mut self,
        support: &mut ProtocolDmabufSupport,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        for backend in &mut self.backends {
            backend.collect_protocol_dmabuf_support(support)?;
        }
        Ok(())
    }
}

/// Overrides `NEKOLAND_BACKEND` for the current thread only.
///
/// This is primarily intended for integration tests that need deterministic backend selection
/// without mutating the whole process environment.
pub fn set_requested_backend_override(requested: Option<String>) -> Option<String> {
    REQUESTED_BACKEND_OVERRIDE.with(|override_cell| override_cell.replace(requested))
}

/// Parse the requested backend list from `NEKOLAND_BACKEND`.
///
/// The variable accepts comma-separated backend names such as `winit`, `drm`,
/// or `virtual`; duplicates are removed while preserving order.
pub fn requested_backend_kinds() -> Vec<BackendKind> {
    let requested = REQUESTED_BACKEND_OVERRIDE
        .with(|override_cell| override_cell.borrow().clone())
        .unwrap_or_else(|| std::env::var("NEKOLAND_BACKEND").unwrap_or_else(|_| "winit".to_owned()));
    let mut kinds = Vec::new();

    for raw_kind in requested.split(',') {
        let kind = match raw_kind.trim() {
            "drm" => BackendKind::Drm,
            "virtual" | "headless" | "offscreen" => BackendKind::Virtual,
            "winit" | "x11" => BackendKind::Winit,
            "" => continue,
            _ => BackendKind::Winit,
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }

    if kinds.is_empty() {
        kinds.push(BackendKind::Winit);
    }

    kinds
}

impl BackendStatus {
    /// Refresh the public status snapshot from the current backend manager.
    pub fn refresh_from_manager(&mut self, manager: &BackendManager) {
        *self = manager.snapshot();
    }
}
