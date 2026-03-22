use std::marker::PhantomData;

use bevy_ecs::error::Result as BevyResult;
use bevy_ecs::prelude::{NonSend, NonSendMut, Res, ResMut};
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::resources::{
    CompiledOutputFrames, CompletedScreenshotFrames, CompositorClock, GlobalPointerPosition,
    PendingScreenshotRequests, PlatformImportDiagnosticsState, PresentAuditState,
    PresentSurfaceSnapshotState, VirtualOutputCaptureState,
};
use nekoland_protocol::ProtocolSurfaceRegistry;

use crate::common::render_order::snapshot_present_audit_outputs;
use crate::manager::SharedBackendManager;
use crate::traits::BackendPresentCtx;

use super::BackendPresentInputs;

#[derive(SystemParam)]
pub(crate) struct BackendPresentState<'w, 's> {
    pub config: Option<Res<'w, CompositorConfig>>,
    pub clock: Option<Res<'w, CompositorClock>>,
    pub pointer: Option<Res<'w, GlobalPointerPosition>>,
    pub present_inputs: Res<'w, BackendPresentInputs>,
    pub present_surfaces: Res<'w, PresentSurfaceSnapshotState>,
    pub compiled_frames: Res<'w, CompiledOutputFrames>,
    pub pending_screenshot_requests: ResMut<'w, PendingScreenshotRequests>,
    pub completed_screenshots: ResMut<'w, CompletedScreenshotFrames>,
    pub import_diagnostics: Option<ResMut<'w, PlatformImportDiagnosticsState>>,
    pub present_audit: ResMut<'w, PresentAuditState>,
    pub surface_registry: Option<NonSend<'w, ProtocolSurfaceRegistry>>,
    pub virtual_output_capture: ResMut<'w, VirtualOutputCaptureState>,
    pub _marker: PhantomData<&'s ()>,
}

/// Let backends present the current render plan using backend-specific surfaces.
pub(crate) fn backend_present_system(
    manager: Option<NonSendMut<SharedBackendManager>>,
    state: BackendPresentState<'_, '_>,
) -> BevyResult {
    let Some(manager) = manager else {
        return Ok(());
    };
    let BackendPresentState {
        config,
        clock,
        pointer,
        present_inputs,
        present_surfaces,
        compiled_frames,
        mut pending_screenshot_requests,
        mut completed_screenshots,
        import_diagnostics,
        mut present_audit,
        surface_registry,
        mut virtual_output_capture,
        ..
    } = state;

    let mut import_diagnostics = import_diagnostics;
    if let Some(diagnostics) = import_diagnostics.as_deref_mut() {
        diagnostics.clear();
    }

    let mut ctx = BackendPresentCtx {
        config: config.as_deref(),
        clock: clock.as_deref(),
        pointer: pointer.as_deref(),
        outputs: &present_inputs.outputs,
        compiled_frames: &compiled_frames,
        pending_screenshot_requests: &mut pending_screenshot_requests,
        completed_screenshots: &mut completed_screenshots,
        surfaces: &present_surfaces.surfaces,
        surface_registry: surface_registry.as_deref(),
        virtual_output_capture: Some(&mut virtual_output_capture),
        import_diagnostics: import_diagnostics.as_deref_mut(),
    };

    let (frame, uptime_millis) = clock
        .as_deref()
        .map(|clock| (clock.frame, clock.uptime_millis.min(u128::from(u64::MAX)) as u64))
        .unwrap_or((0, 0));
    present_audit.outputs = snapshot_present_audit_outputs(
        frame,
        uptime_millis,
        &present_inputs.outputs,
        &compiled_frames,
        &present_surfaces.surfaces,
    );

    manager.borrow_mut().present_all(&mut ctx).map_err(Into::into)
}
