use bevy_app::App;
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
    /// Name of the output last seeded into ECS for this backend.
    seeded_output_name: Option<String>,
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
            seeded_output_name: None,
            presentation_runtime: OutputPresentationRuntime::default(),
            monotonic_clock: None,
        }
    }

    /// Pick the configured virtual-output name, falling back to `Virtual-1`.
    fn desired_output_name(
        &self,
        config: Option<&nekoland_ecs::resources::CompositorConfig>,
    ) -> String {
        config
            .and_then(|config| config.outputs.iter().find(|output| output.enabled))
            .map(|output| output.name.clone())
            .unwrap_or_else(|| "Virtual-1".to_owned())
    }

    /// Iterate output snapshots currently owned by this backend runtime.
    fn owned_outputs<'a>(
        &'a self,
        outputs: &'a [crate::traits::OutputSnapshot],
    ) -> impl Iterator<Item = &'a crate::traits::OutputSnapshot> {
        outputs.iter().filter(|output| output.backend_id == Some(self.id()))
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
        let desired_output_name = self.desired_output_name(cx.config);
        let has_output =
            self.owned_outputs(cx.outputs).any(|output| output.device.name == desired_output_name);
        if !has_output
            && self.seeded_output_name.as_deref() != Some(desired_output_name.as_str())
            && let Some(blueprint) = self.seed_output(&desired_output_name)
        {
            cx.output_events.push(BackendOutputEventRecord {
                backend_id: self.id(),
                output_name: desired_output_name.clone(),
                local_id: blueprint.local_id.clone(),
                change: BackendOutputChange::Connected(blueprint),
            });
            self.seeded_output_name = Some(desired_output_name);
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
            cx.render_graph,
            cx.render_plan,
            cx.materials,
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
            nekoland_ecs::resources::PresentAuditElementKind::SolidRect => {
                VirtualOutputElementKind::SolidRect
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
