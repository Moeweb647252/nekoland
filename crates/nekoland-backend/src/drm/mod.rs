//! DRM backend integrations split by session handling, libinput, device discovery, GBM, and
//! surface rendering.

pub mod device;
pub mod gbm;
pub mod input;
pub mod session;
pub mod surface;

use bevy_app::App;
use smithay::utils::{Clock, Monotonic};

use crate::common::outputs::{
    BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
};
use crate::common::presentation::{OutputPresentationRuntime, emit_present_completion_events};
use crate::traits::{
    Backend, BackendApplyCtx, BackendCapabilities, BackendDescriptor, BackendExtractCtx, BackendId,
    BackendKind, BackendPresentCtx, BackendRole,
};

pub(crate) struct DrmRuntime {
    descriptor: BackendDescriptor,
    session_state: session::SharedDrmSessionState,
    input_state: input::SharedDrmInputState,
    device_state: device::SharedDrmState,
    gbm_state: gbm::SharedGbmState,
    render_state: surface::DrmRenderState,
    presentation_runtime: OutputPresentationRuntime,
    monotonic_clock: Option<Clock<Monotonic>>,
}

impl DrmRuntime {
    pub fn install(app: &mut App, id: BackendId) -> Self {
        let session_state = session::SharedDrmSessionState::default();
        let input_state = input::SharedDrmInputState::default();
        session::install_drm_session_source(app, session_state.clone());
        input::install_drm_input_source(app, session_state.clone(), input_state.clone());

        Self {
            descriptor: BackendDescriptor {
                id,
                kind: BackendKind::Drm,
                role: BackendRole::PrimaryDisplay,
                label: format!("drm-{}", id.0),
                description: "drm backend initializing tty session".to_owned(),
            },
            session_state,
            input_state,
            device_state: device::SharedDrmState::default(),
            gbm_state: gbm::SharedGbmState::default(),
            render_state: surface::DrmRenderState::default(),
            presentation_runtime: OutputPresentationRuntime::default(),
            monotonic_clock: None,
        }
    }

    fn owned_outputs<'a>(
        &'a self,
        outputs: &'a [crate::traits::OutputSnapshot],
    ) -> impl Iterator<Item = &'a crate::traits::OutputSnapshot> {
        outputs.iter().filter(|output| output.backend_id == Some(self.id()))
    }
}

impl Backend for DrmRuntime {
    fn id(&self) -> BackendId {
        self.descriptor.id
    }

    fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.clone()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::INPUT
            | BackendCapabilities::OUTPUT_DISCOVERY
            | BackendCapabilities::OUTPUT_CONFIGURATION
            | BackendCapabilities::PRESENT
            | BackendCapabilities::PRESENT_TIMELINE
    }

    fn seed_output(&self, _output_name: &str) -> Option<BackendOutputBlueprint> {
        None
    }

    fn extract(
        &mut self,
        cx: &mut BackendExtractCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        session::extract_session(session::DrmSessionExtractCtx {
            descriptor: &mut self.descriptor,
            session_state: &self.session_state,
            drm_state: &self.device_state,
            gbm_state: &self.gbm_state,
            input_state: &self.input_state,
            render_state: &mut self.render_state,
            pending_backend_inputs: cx.backend_input_events,
            pending_protocol_inputs: cx.protocol_input_events,
        });

        let connected_connectors =
            device::ensure_drm_device(&self.session_state, &self.device_state);
        for connector in connected_connectors {
            cx.output_events.push(BackendOutputEventRecord {
                backend_id: self.id(),
                output_name: connector.name.clone(),
                change: BackendOutputChange::Connected(
                    connector.output_blueprint(&self.descriptor),
                ),
            });
        }

        gbm::ensure_gbm_allocator(&self.session_state, &self.device_state, &self.gbm_state);
        input::drain_drm_input(
            cx.config,
            self.owned_outputs(cx.outputs)
                .map(|output| output.properties.clone())
                .collect::<Vec<_>>(),
            &self.session_state,
            &self.input_state,
            cx.backend_input_events,
            cx.protocol_input_events,
        );

        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        emit_present_completion_events(
            owned_outputs
                .iter()
                .map(|output| (output.device.name.clone(), output.properties.clone())),
            cx.presentation_events,
            &mut self.presentation_runtime,
            &mut self.monotonic_clock,
        );

        Ok(())
    }

    fn apply(
        &mut self,
        _cx: &mut BackendApplyCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        Ok(())
    }

    fn present(
        &mut self,
        cx: &mut BackendPresentCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        surface::render_drm_outputs(surface::DrmPresentCtx {
            outputs: &owned_outputs,
            config: cx.config,
            cursor_render: cx.cursor_render,
            cursor_image: cx.cursor_image,
            output_damage_regions: cx.output_damage_regions,
            render_list: cx.render_list,
            surfaces: cx.surfaces,
            surface_registry: cx.surface_registry,
            session_state: &self.session_state,
            drm_shared: &self.device_state,
            gbm_shared: &self.gbm_state,
            render_state: &mut self.render_state,
        });
        Ok(())
    }
}
