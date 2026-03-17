use std::collections::{BTreeMap, HashMap};

use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    OutputPresentAudit, PresentAuditElement, PresentAuditElementKind, RenderPlan, RenderPlanItem,
};

use crate::traits::{OutputSnapshot, RenderSurfaceRole, RenderSurfaceSnapshot};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputSurfaceRenderRecord {
    pub surface_id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub opacity: f32,
    pub z_index: i32,
}

pub(crate) fn render_plan_output_surface_records(
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputSurfaceRenderRecord> {
    render_plan
        .outputs
        .get(&output_id)
        .into_iter()
        .flat_map(|output_plan| output_plan.items.iter())
        .filter_map(|item| match item {
            RenderPlanItem::Surface(item) => Some(OutputSurfaceRenderRecord {
                surface_id: item.surface_id,
                x: item.rect.x,
                y: item.rect.y,
                width: item.rect.width,
                height: item.rect.height,
                opacity: item.opacity,
                z_index: item.z_index,
            }),
        })
        .collect()
}

/// Returns output-local surfaces in the front-to-back presentation order expected by backend
/// renderers.
pub(crate) fn render_plan_output_surfaces_in_presentation_order(
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputSurfaceRenderRecord> {
    render_plan
        .outputs
        .get(&output_id)
        .into_iter()
        .flat_map(|output_plan| output_plan.items.iter().rev())
        .filter_map(|item| match item {
            RenderPlanItem::Surface(item) => Some(OutputSurfaceRenderRecord {
                surface_id: item.surface_id,
                x: item.rect.x,
                y: item.rect.y,
                width: item.rect.width,
                height: item.rect.height,
                opacity: item.opacity,
                z_index: item.z_index,
            }),
        })
        .collect()
}

pub(crate) fn render_plan_output_present_audit_elements(
    render_plan: &RenderPlan,
    surfaces: &HashMap<u64, RenderSurfaceSnapshot>,
    output_id: OutputId,
) -> Vec<PresentAuditElement> {
    render_plan_output_surface_records(render_plan, output_id)
        .into_iter()
        .map(|record| PresentAuditElement {
            surface_id: record.surface_id,
            kind: surfaces
                .get(&record.surface_id)
                .map(|surface| present_audit_element_kind(surface.role))
                .unwrap_or(PresentAuditElementKind::Unknown),
            x: record.x,
            y: record.y,
            width: record.width,
            height: record.height,
            z_index: record.z_index,
            opacity: record.opacity,
        })
        .collect()
}

pub(crate) fn snapshot_present_audit_outputs(
    frame: u64,
    uptime_millis: u64,
    outputs: &[OutputSnapshot],
    render_plan: &RenderPlan,
    surfaces: &HashMap<u64, RenderSurfaceSnapshot>,
) -> BTreeMap<OutputId, OutputPresentAudit> {
    outputs
        .iter()
        .map(|output| {
            let elements =
                render_plan_output_present_audit_elements(render_plan, surfaces, output.output_id);
            (
                output.output_id,
                OutputPresentAudit {
                    output_name: output.device.name.clone(),
                    frame,
                    uptime_millis,
                    elements,
                },
            )
        })
        .collect()
}

fn present_audit_element_kind(role: RenderSurfaceRole) -> PresentAuditElementKind {
    match role {
        RenderSurfaceRole::Window => PresentAuditElementKind::Window,
        RenderSurfaceRole::Popup => PresentAuditElementKind::Popup,
        RenderSurfaceRole::Layer => PresentAuditElementKind::Layer,
        RenderSurfaceRole::Unknown => PresentAuditElementKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry,
    };
    use nekoland_ecs::resources::{
        OutputRenderPlan, PresentAuditElementKind, RenderPlan, RenderPlanItem, RenderRect,
        RenderSceneRole, SurfaceRenderItem,
    };

    use crate::traits::{OutputSnapshot, RenderSurfaceRole, RenderSurfaceSnapshot};

    use super::{
        render_plan_output_present_audit_elements, render_plan_output_surface_records,
        render_plan_output_surfaces_in_presentation_order, snapshot_present_audit_outputs,
    };

    #[test]
    fn render_plan_records_drive_present_and_audit_outputs() {
        let surfaces = HashMap::from([
            (
                11,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry { x: 10, y: 20, width: 30, height: 40 },
                    role: RenderSurfaceRole::Window,
                    target_output: Some(OutputId(7)),
                },
            ),
            (
                22,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry { x: 50, y: 60, width: 70, height: 80 },
                    role: RenderSurfaceRole::Layer,
                    target_output: None,
                },
            ),
        ]);
        let output = OutputSnapshot {
            entity: bevy_ecs::entity::Entity::PLACEHOLDER,
            output_id: OutputId(7),
            backend_id: None,
            backend_output_id: None,
            device: OutputDevice {
                name: "HDMI-A-1".to_owned(),
                kind: OutputKind::Nested,
                make: "Nekoland".to_owned(),
                model: "test".to_owned(),
            },
            properties: OutputProperties::default(),
        };
        let render_plan = RenderPlan {
            outputs: HashMap::from([(
                OutputId(7),
                OutputRenderPlan {
                    items: vec![
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            surface_id: 11,
                            rect: RenderRect { x: 10, y: 20, width: 30, height: 40 },
                            opacity: 1.0,
                            z_index: 0,
                            clip_rect: None,
                            scene_role: RenderSceneRole::Desktop,
                        }),
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            surface_id: 22,
                            rect: RenderRect { x: 50, y: 60, width: 70, height: 80 },
                            opacity: 0.5,
                            z_index: 1,
                            clip_rect: None,
                            scene_role: RenderSceneRole::Desktop,
                        }),
                    ],
                },
            )])
            .into_iter()
            .collect(),
        };

        assert_eq!(
            render_plan_output_surface_records(&render_plan, OutputId(7))
                .into_iter()
                .map(|record| record.surface_id)
                .collect::<Vec<_>>(),
            vec![11, 22]
        );
        let elements =
            render_plan_output_present_audit_elements(&render_plan, &surfaces, OutputId(7));
        assert_eq!(elements[0].kind, PresentAuditElementKind::Window);
        assert_eq!(elements[1].kind, PresentAuditElementKind::Layer);
        assert_eq!(
            render_plan_output_surfaces_in_presentation_order(&render_plan, OutputId(7))
                .into_iter()
                .map(|record| record.surface_id)
                .collect::<Vec<_>>(),
            vec![22, 11]
        );

        let audit_outputs =
            snapshot_present_audit_outputs(12, 345, &[output.clone()], &render_plan, &surfaces);
        let audit = &audit_outputs[&OutputId(7)];
        assert_eq!(audit.output_name, "HDMI-A-1");
        assert_eq!(audit.frame, 12);
        assert_eq!(audit.uptime_millis, 345);
        assert_eq!(audit.elements, elements);
    }
}
