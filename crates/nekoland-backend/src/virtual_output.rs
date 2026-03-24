use bevy_app::App;
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
use nekoland_ecs::resources::{VirtualOutputElement, VirtualOutputElementKind, VirtualOutputFrame};
use smithay::utils::{Clock, Monotonic};

use crate::common::outputs::{
    BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
};
use crate::common::presentation::{OutputPresentationRuntime, emit_present_completion_events};
use crate::common::render_order::render_graph_output_present_audit_elements;
use crate::traits::{
    Backend, BackendApplyCtx, BackendCapabilities, BackendDescriptor, BackendExtractCtx, BackendId,
    BackendKind, BackendPresentCtx, BackendRole,
};
const VIRTUAL_PRIMARY_OUTPUT_LOCAL_ID: &str = "primary";

/// Offscreen backend that mirrors the compositor render plan into a synthetic
/// capture stream and presentation timeline.
pub(crate) struct VirtualRuntime {
    /// Public descriptor surfaced through backend status snapshots.
    descriptor: BackendDescriptor,
    /// Per-output sequence/timestamp state for synthetic presentation feedback.
    presentation_runtime: OutputPresentationRuntime,
    /// Monotonic clock used to timestamp synthetic presentation completions.
    monotonic_clock: Option<Clock<Monotonic>>,
}

impl VirtualRuntime {
    /// Install one virtual backend runtime with a deterministic capture-sink descriptor.
    pub fn install(_app: &mut App, id: BackendId) -> Self {
        Self {
            descriptor: BackendDescriptor {
                id,
                kind: BackendKind::Virtual,
                role: BackendRole::CaptureSink,
                label: format!("virtual-{}", id.0),
                description: "offscreen virtual output backend".to_owned(),
            },
            presentation_runtime: OutputPresentationRuntime::default(),
            monotonic_clock: None,
        }
    }

    /// Pick the configured virtual-output name, falling back to `Virtual-1`.
    fn desired_output_name(
        &self,
        config: Option<&CompositorConfig>,
        outputs: &[crate::traits::OutputSnapshot],
        pending_output_events: &[BackendOutputEventRecord],
    ) -> String {
        let configured_output_name = config
            .and_then(|config| config.outputs.iter().find(|output| output.enabled))
            .map(|output| output.name.clone());
        let Some(configured_output_name) = configured_output_name else {
            return "Virtual-1".to_owned();
        };

        if self.output_name_owned_by_other_backend(
            &configured_output_name,
            outputs,
            pending_output_events,
        ) {
            "Virtual-1".to_owned()
        } else {
            configured_output_name
        }
    }

    /// Iterate output snapshots currently owned by this backend runtime.
    fn owned_outputs<'a>(
        &'a self,
        outputs: &'a [crate::traits::OutputSnapshot],
    ) -> impl Iterator<Item = &'a crate::traits::OutputSnapshot> {
        outputs.iter().filter(|output| output.backend_id == Some(self.id()))
    }

    fn output_name_owned_by_other_backend(
        &self,
        output_name: &str,
        outputs: &[crate::traits::OutputSnapshot],
        pending_output_events: &[BackendOutputEventRecord],
    ) -> bool {
        let mut occupied = outputs.iter().any(|output| {
            output.device.name == output_name && output.backend_id != Some(self.id())
        });

        for record in pending_output_events {
            if record.output_name != output_name || record.backend_id == self.id() {
                continue;
            }

            match &record.change {
                BackendOutputChange::Connected(_) => occupied = true,
                BackendOutputChange::Disconnected => occupied = false,
            }
        }

        occupied
    }
}

impl Backend for VirtualRuntime {
    fn id(&self) -> BackendId {
        self.descriptor.id
    }

    fn descriptor(&self) -> BackendDescriptor {
        self.descriptor.clone()
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::OUTPUT_DISCOVERY
            | BackendCapabilities::OUTPUT_CONFIGURATION
            | BackendCapabilities::PRESENT
            | BackendCapabilities::PRESENT_TIMELINE
            | BackendCapabilities::CAPTURE
    }

    fn seed_output(&self, output_name: &str) -> Option<BackendOutputBlueprint> {
        Some(BackendOutputBlueprint {
            local_id: VIRTUAL_PRIMARY_OUTPUT_LOCAL_ID.to_owned(),
            device: OutputDevice {
                name: output_name.to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: self.descriptor.description.clone(),
            },
            properties: OutputProperties {
                width: 1920,
                height: 1080,
                refresh_millihz: 60_000,
                scale: 1,
            },
        })
    }

    fn extract(
        &mut self,
        cx: &mut BackendExtractCtx<'_>,
    ) -> Result<(), nekoland_core::error::NekolandError> {
        // Keep one virtual output materialized so the rest of the compositor can
        // treat the backend like any other output-producing runtime.
        let has_owned_output = self.owned_outputs(cx.outputs).next().is_some();
        let has_pending_owned_connect = cx.output_events.as_slice().iter().any(|record| {
            record.backend_id == self.id()
                && matches!(&record.change, BackendOutputChange::Connected(_))
        });
        if !has_owned_output && !has_pending_owned_connect {
            let desired_output_name =
                self.desired_output_name(cx.config, cx.outputs, cx.output_events.as_slice());
            if let Some(blueprint) = self.seed_output(&desired_output_name) {
                cx.output_events.push(BackendOutputEventRecord {
                    backend_id: self.id(),
                    output_name: desired_output_name,
                    local_id: blueprint.local_id.clone(),
                    change: BackendOutputChange::Connected(blueprint),
                });
            }
        }

        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        emit_present_completion_events(
            owned_outputs.iter().map(|output| (output.output_id, output.properties.clone())),
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
        let Some(capture_state) = cx.virtual_output_capture.as_deref_mut() else {
            return Ok(());
        };
        let owned_outputs = self.owned_outputs(cx.outputs).cloned().collect::<Vec<_>>();
        let Some(output) = owned_outputs.first() else {
            return Ok(());
        };
        let Some(clock) = cx.clock else {
            return Ok(());
        };
        let elements = render_graph_output_present_audit_elements(
            &cx.compiled_frames.render_graph,
            &cx.compiled_frames.render_plan,
            &cx.compiled_frames.materials,
            cx.surfaces,
            output.output_id,
        )
        .into_iter()
        .map(virtual_output_element_from_audit)
        .collect::<Vec<_>>();
        // Serialize the current output-local render plan into a backend-agnostic capture frame so
        // tests and tooling can inspect what would have been presented.

        capture_state.push_frame(VirtualOutputFrame {
            output_name: output.device.name.clone(),
            frame: clock.frame,
            uptime_millis: clock.uptime_millis.min(u128::from(u64::MAX)) as u64,
            width: output.properties.width,
            height: output.properties.height,
            scale: output.properties.scale,
            background_color: cx
                .config
                .map(|config| config.background_color.clone())
                .unwrap_or_else(|| "#000000".to_owned()),
            elements,
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use nekoland_config::resources::{CompositorConfig, ConfiguredOutput};
    use nekoland_ecs::components::{OutputDevice, OutputId, OutputKind, OutputProperties};
    use nekoland_ecs::resources::PendingOutputPresentationEvents;
    use nekoland_ecs::resources::{PendingBackendInputEvents, PendingProtocolInputEvents};

    use crate::common::outputs::{PendingBackendOutputEvents, PendingBackendOutputUpdates};
    use crate::traits::{BackendOutputId, OutputSnapshot};

    use super::{
        Backend, BackendExtractCtx, BackendId, BackendOutputChange, BackendOutputEventRecord,
        VirtualRuntime,
    };

    fn output_snapshot(name: &str, backend_id: BackendId, local_id: &str) -> OutputSnapshot {
        OutputSnapshot {
            output_id: OutputId(1),
            backend_id: Some(backend_id),
            backend_output_id: Some(BackendOutputId { backend_id, local_id: local_id.to_owned() }),
            device: OutputDevice {
                name: name.to_owned(),
                kind: OutputKind::Virtual,
                make: "test".to_owned(),
                model: "test".to_owned(),
            },
            properties: OutputProperties {
                width: 1920,
                height: 1080,
                refresh_millihz: 60_000,
                scale: 1,
            },
        }
    }

    fn configured_output(name: &str) -> CompositorConfig {
        CompositorConfig {
            outputs: vec![ConfiguredOutput {
                name: name.to_owned(),
                mode: "1920x1080@60".to_owned(),
                scale: 1,
                enabled: true,
            }],
            ..CompositorConfig::default()
        }
    }

    #[test]
    fn extract_falls_back_when_configured_name_is_already_live() {
        let mut runtime = VirtualRuntime::install(&mut App::new(), BackendId(9));
        let config = configured_output("eDP-1");
        let outputs = vec![output_snapshot("eDP-1", BackendId(2), "drm-primary")];
        let mut backend_input_events = PendingBackendInputEvents::default();
        let mut protocol_input_events = PendingProtocolInputEvents::default();
        let mut output_events = PendingBackendOutputEvents::default();
        let mut output_updates = PendingBackendOutputUpdates::default();
        let mut presentation_events = PendingOutputPresentationEvents::default();
        let mut cx = BackendExtractCtx {
            app_metadata: None,
            config: Some(&config),
            outputs: &outputs,
            backend_input_events: &mut backend_input_events,
            protocol_input_events: &mut protocol_input_events,
            output_events: &mut output_events,
            output_updates: &mut output_updates,
            presentation_events: &mut presentation_events,
            winit_window_state: None,
        };

        runtime.extract(&mut cx).expect("virtual extract should succeed");

        let Some(record) = output_events.as_slice().last() else {
            panic!("virtual backend should seed an output");
        };
        assert_eq!(record.output_name, "Virtual-1");
    }

    #[test]
    fn extract_respects_pending_other_backend_connects() {
        let mut runtime = VirtualRuntime::install(&mut App::new(), BackendId(9));
        let config = configured_output("eDP-1");
        let outputs = Vec::new();
        let mut backend_input_events = PendingBackendInputEvents::default();
        let mut protocol_input_events = PendingProtocolInputEvents::default();
        let mut output_events =
            PendingBackendOutputEvents::from_items(vec![BackendOutputEventRecord {
                backend_id: BackendId(2),
                output_name: "eDP-1".to_owned(),
                local_id: "drm-primary".to_owned(),
                change: BackendOutputChange::Connected(super::BackendOutputBlueprint {
                    local_id: "drm-primary".to_owned(),
                    device: OutputDevice {
                        name: "eDP-1".to_owned(),
                        kind: OutputKind::Physical,
                        make: "test".to_owned(),
                        model: "panel".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1920,
                        height: 1080,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                }),
            }]);
        let mut output_updates = PendingBackendOutputUpdates::default();
        let mut presentation_events = PendingOutputPresentationEvents::default();
        let mut cx = BackendExtractCtx {
            app_metadata: None,
            config: Some(&config),
            outputs: &outputs,
            backend_input_events: &mut backend_input_events,
            protocol_input_events: &mut protocol_input_events,
            output_events: &mut output_events,
            output_updates: &mut output_updates,
            presentation_events: &mut presentation_events,
            winit_window_state: None,
        };

        runtime.extract(&mut cx).expect("virtual extract should succeed");

        let Some(record) = output_events.as_slice().last() else {
            panic!("virtual backend should seed an output");
        };
        assert_eq!(record.backend_id, BackendId(9));
        assert_eq!(record.output_name, "Virtual-1");
    }
}

fn virtual_output_element_from_audit(
    element: nekoland_ecs::resources::PresentAuditElement,
) -> VirtualOutputElement {
    VirtualOutputElement {
        surface_id: element.surface_id,
        kind: match element.kind {
            nekoland_ecs::resources::PresentAuditElementKind::Window => {
                VirtualOutputElementKind::Window
            }
            nekoland_ecs::resources::PresentAuditElementKind::Popup => {
                VirtualOutputElementKind::Popup
            }
            nekoland_ecs::resources::PresentAuditElementKind::Layer => {
                VirtualOutputElementKind::Layer
            }
            nekoland_ecs::resources::PresentAuditElementKind::Quad => {
                VirtualOutputElementKind::Quad
            }
            nekoland_ecs::resources::PresentAuditElementKind::Backdrop => {
                VirtualOutputElementKind::Backdrop
            }
            nekoland_ecs::resources::PresentAuditElementKind::Compositor => {
                VirtualOutputElementKind::Compositor
            }
            nekoland_ecs::resources::PresentAuditElementKind::Cursor => {
                VirtualOutputElementKind::Cursor
            }
            nekoland_ecs::resources::PresentAuditElementKind::Unknown => {
                VirtualOutputElementKind::Unknown
            }
        },
        x: element.x,
        y: element.y,
        width: element.width,
        height: element.height,
        z_index: element.z_index,
        opacity: element.opacity,
    }
}
