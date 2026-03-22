use bevy_ecs::prelude::{NonSend, Res, ResMut};

use crate::manager::{BackendStatus, SharedBackendManager};

pub(super) fn sync_backend_wayland_feedback_system(
    pending_screenshot_requests: Res<'_, nekoland_ecs::resources::PendingScreenshotRequests>,
    completed_screenshots: Res<'_, nekoland_ecs::resources::CompletedScreenshotFrames>,
    backend_status: Res<'_, BackendStatus>,
    import_diagnostics: Res<'_, nekoland_ecs::resources::PlatformImportDiagnosticsState>,
    output_presentation: Res<'_, nekoland_ecs::resources::OutputPresentationState>,
    present_audit: Res<'_, nekoland_ecs::resources::PresentAuditState>,
    virtual_output_capture: Res<'_, nekoland_ecs::resources::VirtualOutputCaptureState>,
    mut wayland_feedback: ResMut<'_, nekoland_ecs::resources::WaylandFeedback>,
) {
    wayland_feedback.platform_backends = backend_status.platform_state();
    wayland_feedback.import_diagnostics = import_diagnostics.clone();
    wayland_feedback.pending_screenshot_requests = pending_screenshot_requests.clone();
    wayland_feedback.completed_screenshots = completed_screenshots.clone();
    wayland_feedback.output_presentation = output_presentation.clone();
    wayland_feedback.present_audit = present_audit.clone();
    wayland_feedback.virtual_output_capture = virtual_output_capture.clone();
}

pub(super) fn clear_backend_frame_local_queues_system(
    mut pending_backend_inputs: ResMut<'_, nekoland_ecs::resources::PendingBackendInputEvents>,
    mut pending_protocol_inputs: ResMut<'_, nekoland_ecs::resources::PendingProtocolInputEvents>,
    mut pending_output_events: ResMut<'_, crate::common::outputs::PendingBackendOutputEvents>,
    mut pending_output_updates: ResMut<'_, crate::common::outputs::PendingBackendOutputUpdates>,
) {
    *pending_backend_inputs = nekoland_ecs::resources::PendingBackendInputEvents::default();
    *pending_protocol_inputs = nekoland_ecs::resources::PendingProtocolInputEvents::default();
    *pending_output_events = crate::common::outputs::PendingBackendOutputEvents::default();
    *pending_output_updates = crate::common::outputs::PendingBackendOutputUpdates::default();
}

/// Refresh the public backend-status resource from the installed backend manager.
pub(super) fn sync_backend_status_system(
    manager: Option<NonSend<SharedBackendManager>>,
    mut status: ResMut<BackendStatus>,
) {
    let Some(manager) = manager else {
        return;
    };
    status.refresh_from_manager(&manager.borrow());
}
