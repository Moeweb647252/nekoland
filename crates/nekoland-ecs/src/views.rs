use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::query::QueryData;

use crate::components::{
    BufferState, LayerShellSurface, OutputBackgroundWindow, OutputCurrentWorkspace, OutputDevice,
    OutputPlacement, OutputProperties, OutputViewport, OutputWorkArea, PopupGrab,
    SurfaceContentVersion, SurfaceGeometry, WindowFullscreenTarget, WindowLayout, WindowMode,
    WindowPlacement, WindowPolicyState, WindowRestoreSnapshot, WindowRole, WindowSceneGeometry,
    WindowViewportVisibility, WlSurfaceHandle, Workspace, X11Window, XdgPopup, XdgWindow,
};

/// Common read-only runtime view over one surface-backed entity with committed geometry.
#[derive(QueryData)]
pub struct SurfaceRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub geometry: &'static SurfaceGeometry,
    pub content_version: &'static SurfaceContentVersion,
}

impl<'w, 's> SurfaceRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Read-only view for focus/hover decisions over visible windows.
#[derive(QueryData)]
pub struct WindowFocusRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub geometry: &'static SurfaceGeometry,
    pub viewport_visibility: &'static WindowViewportVisibility,
    pub role: &'static WindowRole,
    pub background: Option<&'static OutputBackgroundWindow>,
    pub layout: &'static WindowLayout,
    pub mode: &'static WindowMode,
    pub child_of: Option<&'static ChildOf>,
    pub x11_window: Option<&'static X11Window>,
}

impl<'w, 's> WindowFocusRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Common mutable runtime view over one managed window entity.
///
/// This keeps frequently co-accessed window runtime components grouped together so shell/layout
/// systems do not have to repeat long tuple queries.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct WindowRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub geometry: &'static mut SurfaceGeometry,
    pub scene_geometry: &'static mut WindowSceneGeometry,
    pub content_version: &'static mut SurfaceContentVersion,
    pub viewport_visibility: &'static mut WindowViewportVisibility,
    pub role: &'static mut WindowRole,
    pub background: Option<&'static mut OutputBackgroundWindow>,
    pub placement: &'static mut WindowPlacement,
    pub fullscreen_target: &'static mut WindowFullscreenTarget,
    pub restore: &'static mut WindowRestoreSnapshot,
    pub policy_state: &'static mut WindowPolicyState,
    pub layout: &'static mut WindowLayout,
    pub mode: &'static mut WindowMode,
    pub child_of: Option<&'static ChildOf>,
    pub buffer: Option<&'static mut BufferState>,
    pub xdg_window: Option<&'static mut XdgWindow>,
    pub x11_window: Option<&'static mut X11Window>,
}

impl<'w, 's> WindowRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }

    pub fn has_explicit_placement(&self) -> bool {
        self.placement.has_explicit_placement()
    }
}

impl<'w, 's> WindowRuntimeReadOnlyItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }

    pub fn has_explicit_placement(&self) -> bool {
        self.placement.has_explicit_placement()
    }
}

/// Lightweight read-only view for visibility-oriented window queries.
#[derive(QueryData)]
pub struct WindowVisibilityRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub viewport_visibility: &'static WindowViewportVisibility,
    pub role: &'static WindowRole,
    pub background: Option<&'static OutputBackgroundWindow>,
    pub mode: &'static WindowMode,
}

impl<'w, 's> WindowVisibilityRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Lightweight read-only view for popup visibility and parentage checks.
#[derive(QueryData)]
pub struct PopupRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub buffer: &'static BufferState,
    pub child_of: &'static ChildOf,
}

impl<'w, 's> PopupRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Mutable popup view used when protocol configure/grab state needs to be updated.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct PopupConfigureRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub popup: &'static mut XdgPopup,
    pub grab: Option<&'static mut PopupGrab>,
}

impl<'w, 's> PopupConfigureRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Read-only view for window snapshots exported to IPC or other introspection paths.
#[derive(QueryData)]
pub struct WindowSnapshotRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub window: &'static XdgWindow,
    pub x11_window: Option<&'static X11Window>,
    pub geometry: &'static SurfaceGeometry,
    pub scene_geometry: &'static WindowSceneGeometry,
    pub content_version: &'static SurfaceContentVersion,
    pub viewport_visibility: &'static WindowViewportVisibility,
    pub role: &'static WindowRole,
    pub background: Option<&'static OutputBackgroundWindow>,
    pub mode: &'static WindowMode,
    pub layout: &'static WindowLayout,
    pub child_of: Option<&'static ChildOf>,
}

impl<'w, 's> WindowSnapshotRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Read-only view for popup snapshots exported to IPC or visibility logic.
#[derive(QueryData)]
pub struct PopupSnapshotRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub child_of: &'static ChildOf,
    pub geometry: &'static SurfaceGeometry,
    pub buffer: &'static BufferState,
    pub content_version: &'static SurfaceContentVersion,
    pub grab: Option<&'static PopupGrab>,
}

impl<'w, 's> PopupSnapshotRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Read-only view for frame composition over toplevel windows.
#[derive(QueryData)]
pub struct WindowRenderRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub viewport_visibility: &'static WindowViewportVisibility,
    pub role: &'static WindowRole,
    pub background: Option<&'static OutputBackgroundWindow>,
    pub mode: &'static WindowMode,
    pub child_of: Option<&'static ChildOf>,
}

impl<'w, 's> WindowRenderRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Read-only view for frame composition over popups.
#[derive(QueryData)]
pub struct PopupRenderRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub buffer: &'static BufferState,
    pub child_of: &'static ChildOf,
}

impl<'w, 's> PopupRenderRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Read-only view for frame composition over layer-shell surfaces.
#[derive(QueryData)]
pub struct LayerRenderRuntime {
    pub surface: &'static WlSurfaceHandle,
    pub layer_surface: &'static LayerShellSurface,
    pub buffer: &'static BufferState,
    pub content_version: &'static SurfaceContentVersion,
}

impl<'w, 's> LayerRenderRuntimeItem<'w, 's> {
    pub fn surface_id(&self) -> u64 {
        self.surface.id
    }
}

/// Common runtime view over one workspace entity.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct WorkspaceRuntime {
    pub workspace: &'static mut Workspace,
}

impl<'w, 's> WorkspaceRuntimeItem<'w, 's> {
    pub fn id(&self) -> crate::components::WorkspaceId {
        self.workspace.id
    }

    pub fn name(&self) -> &str {
        &self.workspace.name
    }

    pub fn is_active(&self) -> bool {
        self.workspace.active
    }
}

impl<'w, 's> WorkspaceRuntimeReadOnlyItem<'w, 's> {
    pub fn id(&self) -> crate::components::WorkspaceId {
        self.workspace.id
    }

    pub fn name(&self) -> &str {
        &self.workspace.name
    }

    pub fn is_active(&self) -> bool {
        self.workspace.active
    }
}

/// Common runtime view over one output entity.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OutputRuntime {
    pub device: &'static OutputDevice,
    pub properties: &'static mut OutputProperties,
    pub placement: &'static mut OutputPlacement,
    pub viewport: &'static mut OutputViewport,
    pub work_area: &'static mut OutputWorkArea,
    pub current_workspace: Option<&'static mut OutputCurrentWorkspace>,
}

impl<'w, 's> OutputRuntimeItem<'w, 's> {
    pub fn name(&self) -> &str {
        &self.device.name
    }
}

impl<'w, 's> OutputRuntimeReadOnlyItem<'w, 's> {
    pub fn name(&self) -> &str {
        &self.device.name
    }
}
