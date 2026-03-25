//! Backend plugin wiring for main-world reconciliation and Wayland-subapp runtime execution.

use bevy_app::App;
use bevy_ecs::prelude::{Entity, Query, Resource};
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use nekoland_core::app::{
    WaylandApplySystems, WaylandCleanupSystems, WaylandExtractSystems, WaylandFeedbackSystems,
    WaylandNormalizeSystems, WaylandPresentSystems,
};
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule, ProtocolSchedule};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::resources::{
    BackendOutputRegistry, CompiledOutputFrames, CompletedScreenshotFrames, FocusedOutputState,
    OutputViewportAnimationState, PendingBackendInputEvents, PendingProtocolInputEvents,
    PendingScreenshotRequests, PlatformImportDiagnosticsState, PresentAuditState,
    PresentSurfaceSnapshotState, ShellRenderInput, ViewportAnimationActivityState,
    VirtualOutputCaptureState,
};
use nekoland_ecs::views::{BackendPresentSurfaceRuntime, OutputRuntime};
use nekoland_protocol::{
    ProtocolDmabufSupport, ProtocolSeatDispatchSystems, resources::PendingOutputPresentationEvents,
};

use crate::common::outputs::{
    PendingBackendOutputEvents, PendingBackendOutputUpdates, RememberedOutputViewportState,
    advance_output_viewport_animations_system, apply_output_control_requests_system,
    apply_output_overlay_controls_system, apply_output_server_requests_system,
    remember_output_viewports_system, sync_configured_outputs_system,
    sync_output_layout_state_system, sync_output_snapshot_state_from_present_inputs_system,
};
use crate::common::presentation::apply_output_presentation_events_system;
use crate::components::OutputBackend;
use crate::manager::{BackendManager, BackendStatus, SharedBackendManager};

/// Wayland-subapp apply phase for backend runtimes.
pub mod apply;
/// Main-world to Wayland-subapp extraction helpers for backend resources.
pub mod extract;
/// Backend status and feedback synchronization helpers.
pub mod feedback;
/// Normalization of backend-owned state into shared ECS resources.
pub mod normalize;
/// Final backend present execution.
pub mod present;

#[derive(Debug, Default, Clone, Copy)]
/// Main-world plugin that owns backend-facing output reconciliation resources.
pub struct BackendPlugin;

#[derive(Debug, Default, Clone, Copy)]
/// Wayland-subapp plugin that runs backend extraction, apply, and present phases.
pub struct BackendWaylandSubAppPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Present-phase set reserved for backend submission work.
pub struct BackendPresentSystems;

type BackendOutputQuery<'w, 's> =
    Query<'w, 's, (Entity, OutputRuntime, Option<&'static OutputBackend>)>;
type BackendPresentSurfaceQuery<'w, 's> = Query<'w, 's, (Entity, BackendPresentSurfaceRuntime)>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
/// Normalized output snapshots exposed to backend extract and present systems.
pub struct BackendPresentInputs {
    outputs: Vec<crate::traits::OutputSnapshot>,
}

impl BackendPresentInputs {
    /// Builds backend present inputs from the provided output snapshots.
    pub fn from_outputs(outputs: Vec<crate::traits::OutputSnapshot>) -> Self {
        Self { outputs }
    }

    /// Returns the normalized output snapshots currently visible to backend runtimes.
    pub fn outputs(&self) -> &[crate::traits::OutputSnapshot] {
        &self.outputs
    }
}

impl NekolandPlugin for BackendPlugin {
    /// Register backend resources plus the extract/apply/present pipeline that
    /// keeps runtime backends in sync with ECS state.
    fn build(&self, app: &mut App) {
        app.insert_resource(BackendOutputRegistry::default())
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(OutputViewportAnimationState::default())
            .insert_resource(ViewportAnimationActivityState::default())
            .init_resource::<FocusedOutputState>()
            .init_resource::<PendingOutputPresentationEvents>()
            .add_message::<OutputConnected>()
            .add_message::<OutputDisconnected>()
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    advance_output_viewport_animations_system,
                    apply_output_overlay_controls_system,
                    sync_output_layout_state_system,
                    remember_output_viewports_system,
                )
                    .chain(),
            )
            .configure_sets(
                PresentSchedule,
                BackendPresentSystems.after(ProtocolSeatDispatchSystems),
            );
    }
}

impl NekolandPlugin for BackendWaylandSubAppPlugin {
    fn build(&self, app: &mut App) {
        let manager = SharedBackendManager::new(BackendManager::bootstrap(app));
        app.insert_non_send_resource(manager)
            .init_resource::<BackendStatus>()
            .init_resource::<BackendPresentInputs>()
            .init_resource::<PlatformImportDiagnosticsState>()
            .init_resource::<ProtocolDmabufSupport>()
            .init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingBackendOutputEvents>()
            .init_resource::<PendingBackendOutputUpdates>()
            .init_resource::<PendingOutputPresentationEvents>()
            .init_resource::<PresentAuditState>()
            .init_resource::<PresentSurfaceSnapshotState>()
            .init_resource::<ShellRenderInput>()
            .init_resource::<PendingScreenshotRequests>()
            .init_resource::<CompletedScreenshotFrames>()
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<crate::winit::backend::WinitWindowState>()
            .add_systems(
                ExtractSchedule,
                (extract::sync_protocol_dmabuf_support_system, extract::backend_extract_system)
                    .chain()
                    .in_set(WaylandExtractSystems),
            )
            .add_systems(
                ExtractSchedule,
                (
                    normalize::sync_platform_input_events_from_backend_inputs_system,
                    sync_output_snapshot_state_from_present_inputs_system,
                )
                    .chain()
                    .in_set(WaylandNormalizeSystems),
            )
            .add_systems(
                ProtocolSchedule,
                (apply::backend_apply_system, feedback::sync_backend_status_system)
                    .chain()
                    .in_set(WaylandApplySystems),
            )
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_presentation_events_system,
                    sync_configured_outputs_system,
                    apply_output_server_requests_system,
                    normalize::sync_backend_wayland_ingress_system,
                )
                    .chain()
                    .in_set(WaylandApplySystems),
            )
            .add_systems(
                PresentSchedule,
                present::backend_present_system
                    .in_set(BackendPresentSystems)
                    .in_set(WaylandPresentSystems),
            )
            .add_systems(
                PresentSchedule,
                feedback::sync_backend_wayland_feedback_system
                    .after(BackendPresentSystems)
                    .in_set(WaylandFeedbackSystems),
            )
            .add_systems(
                PresentSchedule,
                feedback::clear_backend_frame_local_queues_system.in_set(WaylandCleanupSystems),
            );
    }
}

#[cfg(test)]
mod tests;
