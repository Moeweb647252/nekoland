use std::collections::{BTreeMap, HashMap};

use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    OutputPresentAudit, PresentAuditElement, PresentAuditElementKind, RenderColor,
    RenderItemInstance, RenderPassGraph, RenderPassKind, RenderPlan, RenderPlanItem,
};

use crate::traits::{OutputSnapshot, RenderSurfaceRole, RenderSurfaceSnapshot};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputSurfaceRenderRecord {
    pub surface_id: u64,
    pub instance: RenderItemInstance,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputSolidRectRenderRecord {
    pub color: RenderColor,
    pub instance: RenderItemInstance,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputBackdropRenderRecord {
    pub instance: RenderItemInstance,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OutputRenderRecord {
    Surface(OutputSurfaceRenderRecord),
    SolidRect(OutputSolidRectRenderRecord),
    Backdrop(OutputBackdropRenderRecord),
}

pub(crate) fn render_graph_output_records(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputRenderRecord> {
    let Some(execution) = render_graph.outputs.get(&output_id) else {
        return Vec::new();
    };
    let Some(output_plan) = render_plan.outputs.get(&output_id) else {
        return Vec::new();
    };

    execution
        .reachable_passes_in_order()
        .into_iter()
        .filter_map(|pass_id| execution.passes.get(&pass_id))
        .filter(|pass| pass.kind == RenderPassKind::Scene)
        .flat_map(|pass| pass.item_indices.iter())
        .filter_map(|item_index| output_plan.items.get(*item_index))
        .map(output_record_from_plan_item)
        .collect::<Vec<_>>()
}

/// Returns output-local surfaces in the front-to-back presentation order expected by backend
/// renderers.
pub(crate) fn render_graph_output_surfaces_in_presentation_order(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputSurfaceRenderRecord> {
    let Some(execution) = render_graph.outputs.get(&output_id) else {
        return Vec::new();
    };
    let Some(output_plan) = render_plan.outputs.get(&output_id) else {
        return Vec::new();
    };

    execution
        .reachable_passes_in_order()
        .into_iter()
        .rev()
        .filter_map(|pass_id| execution.passes.get(&pass_id))
        .filter(|pass| pass.kind == RenderPassKind::Scene)
        .flat_map(|pass| pass.item_indices.iter().rev())
        .filter_map(|item_index| output_plan.items.get(*item_index))
        .filter_map(|item| match item {
            RenderPlanItem::Surface(item) => Some(OutputSurfaceRenderRecord {
                surface_id: item.surface_id,
                instance: item.instance,
            }),
            RenderPlanItem::SolidRect(_) | RenderPlanItem::Backdrop(_) => None,
        })
        .collect::<Vec<_>>()
}

pub(crate) fn render_graph_output_present_audit_elements(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    surfaces: &HashMap<u64, RenderSurfaceSnapshot>,
    output_id: OutputId,
) -> Vec<PresentAuditElement> {
    render_graph_output_records(render_graph, render_plan, output_id)
        .into_iter()
        .filter_map(|record| match record {
            OutputRenderRecord::Surface(record) => {
                let visible_rect = record.instance.visible_rect()?;
                Some(PresentAuditElement {
                    surface_id: record.surface_id,
                    kind: surfaces
                        .get(&record.surface_id)
                        .map(|surface| present_audit_element_kind(surface.role))
                        .unwrap_or(PresentAuditElementKind::Unknown),
                    x: visible_rect.x,
                    y: visible_rect.y,
                    width: visible_rect.width,
                    height: visible_rect.height,
                    z_index: record.instance.z_index,
                    opacity: record.instance.opacity,
                })
            }
            OutputRenderRecord::SolidRect(_) | OutputRenderRecord::Backdrop(_) => None,
        })
        .collect()
}

pub(crate) fn snapshot_present_audit_outputs(
    frame: u64,
    uptime_millis: u64,
    outputs: &[OutputSnapshot],
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    surfaces: &HashMap<u64, RenderSurfaceSnapshot>,
) -> BTreeMap<OutputId, OutputPresentAudit> {
    outputs
        .iter()
        .map(|output| {
            let elements = render_graph_output_present_audit_elements(
                render_graph,
                render_plan,
                surfaces,
                output.output_id,
            );
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

fn output_record_from_plan_item(item: &RenderPlanItem) -> OutputRenderRecord {
    match item {
        RenderPlanItem::Surface(item) => OutputRenderRecord::Surface(OutputSurfaceRenderRecord {
            surface_id: item.surface_id,
            instance: item.instance,
        }),
        RenderPlanItem::SolidRect(item) => {
            OutputRenderRecord::SolidRect(OutputSolidRectRenderRecord {
                color: item.color,
                instance: item.instance,
            })
        }
        RenderPlanItem::Backdrop(item) => {
            OutputRenderRecord::Backdrop(OutputBackdropRenderRecord { instance: item.instance })
        }
    }
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
        OutputExecutionPlan, OutputRenderPlan, PresentAuditElementKind, RenderItemInstance,
        RenderPassGraph, RenderPassId, RenderPassNode, RenderPlan, RenderPlanItem, RenderRect,
        RenderSceneRole, RenderTargetId, RenderTargetKind, SurfaceRenderItem,
    };

    use crate::traits::{OutputSnapshot, RenderSurfaceRole, RenderSurfaceSnapshot};

    use super::{
        render_graph_output_present_audit_elements, render_graph_output_records,
        render_graph_output_surfaces_in_presentation_order, snapshot_present_audit_outputs,
    };

    #[test]
    fn render_graph_records_drive_present_and_audit_outputs() {
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
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 10, y: 20, width: 30, height: 40 },
                                opacity: 1.0,
                                clip_rect: None,
                                z_index: 0,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            surface_id: 22,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 50, y: 60, width: 70, height: 80 },
                                opacity: 0.5,
                                clip_rect: None,
                                z_index: 1,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                    ],
                },
            )])
            .into_iter()
            .collect(),
        };
        let render_graph = RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                OutputId(7),
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(OutputId(7)),
                    )]),
                    passes: std::collections::BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::scene(
                            RenderSceneRole::Desktop,
                            RenderTargetId(1),
                            Vec::new(),
                            vec![0, 1],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                },
            )]),
        };

        assert_eq!(
            render_graph_output_records(&render_graph, &render_plan, OutputId(7))
                .into_iter()
                .filter_map(|record| match record {
                    super::OutputRenderRecord::Surface(record) => Some(record.surface_id),
                    super::OutputRenderRecord::SolidRect(_)
                    | super::OutputRenderRecord::Backdrop(_) => None,
                })
                .collect::<Vec<_>>(),
            vec![11, 22]
        );
        let elements = render_graph_output_present_audit_elements(
            &render_graph,
            &render_plan,
            &surfaces,
            OutputId(7),
        );
        assert_eq!(elements[0].kind, PresentAuditElementKind::Window);
        assert_eq!(elements[1].kind, PresentAuditElementKind::Layer);
        assert_eq!(
            render_graph_output_surfaces_in_presentation_order(
                &render_graph,
                &render_plan,
                OutputId(7),
            )
            .into_iter()
            .map(|record| record.surface_id)
            .collect::<Vec<_>>(),
            vec![22, 11]
        );

        let audit_outputs = snapshot_present_audit_outputs(
            12,
            345,
            &[output.clone()],
            &render_graph,
            &render_plan,
            &surfaces,
        );
        let audit = &audit_outputs[&OutputId(7)];
        assert_eq!(audit.output_name, "HDMI-A-1");
        assert_eq!(audit.frame, 12);
        assert_eq!(audit.uptime_millis, 345);
        assert_eq!(audit.elements, elements);
    }
}
