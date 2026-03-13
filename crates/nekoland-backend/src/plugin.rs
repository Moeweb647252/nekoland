use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{Entity, NonSend, NonSendMut, Query, Res, ResMut};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::prelude::AppMetadata;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule};
use nekoland_ecs::components::{
    LayerShellSurface, SurfaceGeometry, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::resources::{
    CompositorClock, CompositorConfig, GlobalPointerPosition, OutputPresentationState,
    PendingBackendInputEvents, PendingOutputControls, PendingOutputPresentationEvents,
    PendingOutputServerRequests, PendingProtocolInputEvents, PrimaryOutputState, RenderList,
    VirtualOutputCaptureState,
};
use nekoland_ecs::views::OutputRuntime;
use nekoland_protocol::ProtocolSurfaceRegistry;

use crate::common::outputs::{
    BackendOutputRegistry, PendingBackendOutputEvents, PendingBackendOutputUpdates,
    apply_backend_output_updates_system, apply_output_control_requests_system,
    apply_output_server_requests_system, collect_output_snapshots, sync_configured_outputs_system,
    sync_primary_output_state_system, synchronize_backend_outputs_system,
};
use crate::common::presentation::apply_output_presentation_events_system;
use crate::components::OutputBackend;
use crate::manager::{BackendManager, BackendStatus};
use crate::traits::{
    BackendApplyCtx, BackendExtractCtx, BackendPresentCtx, RenderSurfaceRole, RenderSurfaceSnapshot,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct BackendPlugin;

impl NekolandPlugin for BackendPlugin {
    /// Register backend resources plus the extract/apply/present pipeline that
    /// keeps runtime backends in sync with ECS state.
    fn build(&self, app: &mut App) {
        let manager = BackendManager::bootstrap(app);

        app.insert_resource(BackendStatus::default())
            .insert_resource(BackendOutputRegistry::default())
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<PendingOutputPresentationEvents>()
            .init_resource::<OutputPresentationState>()
            .init_resource::<PrimaryOutputState>()
            .init_resource::<PendingBackendOutputEvents>()
            .init_resource::<PendingBackendOutputUpdates>()
            .insert_non_send_resource(manager)
            .add_message::<OutputConnected>()
            .add_message::<OutputDisconnected>()
            .add_systems(
                ExtractSchedule,
                (
                    sync_configured_outputs_system,
                    backend_extract_system,
                    synchronize_backend_outputs_system,
                    apply_backend_output_updates_system,
                    apply_output_control_requests_system,
                    apply_output_server_requests_system,
                    sync_primary_output_state_system,
                    backend_apply_system,
                    sync_backend_status_system,
                    apply_output_presentation_events_system,
                )
                    .chain(),
            )
            .add_systems(PresentSchedule, backend_present_system);
    }
}

/// Collect backend-originated events and state updates into ECS pending queues.
fn backend_extract_system(
    mut manager: NonSendMut<BackendManager>,
    app_metadata: Option<Res<AppMetadata>>,
    config: Option<Res<CompositorConfig>>,
    outputs: Query<(Entity, OutputRuntime, Option<&OutputBackend>)>,
    mut pending_backend_inputs: ResMut<PendingBackendInputEvents>,
    mut pending_protocol_inputs: ResMut<PendingProtocolInputEvents>,
    mut pending_output_events: ResMut<PendingBackendOutputEvents>,
    mut pending_output_updates: ResMut<PendingBackendOutputUpdates>,
    mut pending_presentation_events: ResMut<PendingOutputPresentationEvents>,
    winit_window_state: Option<ResMut<crate::winit::backend::WinitWindowState>>,
) -> BevyResult {
    let output_snapshots = collect_output_snapshots(&outputs);
    let mut winit_window_state = winit_window_state;
    let mut ctx = BackendExtractCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &output_snapshots,
        backend_input_events: &mut pending_backend_inputs,
        protocol_input_events: &mut pending_protocol_inputs,
        output_events: &mut pending_output_events,
        output_updates: &mut pending_output_updates,
        presentation_events: &mut pending_presentation_events,
        winit_window_state: winit_window_state.as_mut().map(|state| &mut **state),
    };

    manager.extract_all(&mut ctx).map_err(Into::into)
}

/// Let backends consume already-normalized ECS state before presentation.
fn backend_apply_system(
    mut manager: NonSendMut<BackendManager>,
    app_metadata: Option<Res<AppMetadata>>,
    config: Option<Res<CompositorConfig>>,
    outputs: Query<(Entity, OutputRuntime, Option<&OutputBackend>)>,
    winit_window_state: Option<ResMut<crate::winit::backend::WinitWindowState>>,
) -> BevyResult {
    let output_snapshots = collect_output_snapshots(&outputs);
    let mut winit_window_state = winit_window_state;
    let mut ctx = BackendApplyCtx {
        app_metadata: app_metadata.as_deref(),
        config: config.as_deref(),
        outputs: &output_snapshots,
        winit_window_state: winit_window_state.as_mut().map(|state| &mut **state),
    };

    manager.apply_all(&mut ctx).map_err(Into::into)
}

/// Let backends present the current render list using backend-specific surfaces.
fn backend_present_system(
    mut manager: NonSendMut<BackendManager>,
    config: Option<Res<CompositorConfig>>,
    clock: Option<Res<CompositorClock>>,
    pointer: Option<Res<GlobalPointerPosition>>,
    outputs: Query<(Entity, OutputRuntime, Option<&OutputBackend>)>,
    surfaces: Query<(
        &WlSurfaceHandle,
        &SurfaceGeometry,
        Option<&XdgWindow>,
        Option<&XdgPopup>,
        Option<&LayerShellSurface>,
    )>,
    render_list: Res<RenderList>,
    surface_registry: Option<NonSend<ProtocolSurfaceRegistry>>,
    mut virtual_output_capture: ResMut<VirtualOutputCaptureState>,
) -> BevyResult {
    let output_snapshots = collect_output_snapshots(&outputs);
    let surface_snapshots = surfaces
        .iter()
        .map(|(surface, geometry, window, popup, layer)| {
            let role = if window.is_some() {
                RenderSurfaceRole::Window
            } else if popup.is_some() {
                RenderSurfaceRole::Popup
            } else if layer.is_some() {
                RenderSurfaceRole::Layer
            } else {
                RenderSurfaceRole::Unknown
            };
            (surface.id, RenderSurfaceSnapshot { geometry: geometry.clone(), role })
        })
        .collect::<HashMap<_, _>>();

    let mut ctx = BackendPresentCtx {
        config: config.as_deref(),
        clock: clock.as_deref(),
        pointer: pointer.as_deref(),
        outputs: &output_snapshots,
        render_list: &render_list,
        surfaces: &surface_snapshots,
        surface_registry: surface_registry.as_deref(),
        virtual_output_capture: Some(&mut virtual_output_capture),
    };

    manager.present_all(&mut ctx).map_err(Into::into)
}

/// Refresh the public backend-status resource from the installed backend manager.
fn sync_backend_status_system(manager: NonSend<BackendManager>, mut status: ResMut<BackendStatus>) {
    status.refresh_from_manager(&manager);
}
