use std::collections::{BTreeMap, HashMap};

use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    OutputExecutionPlan, OutputPresentAudit, PresentAuditElement, PresentAuditElementKind,
    RenderColor, RenderItemInstance, RenderPassGraph, RenderPassKind, RenderPlan, RenderPlanItem,
    RenderSceneRole, RenderTargetId, RenderTargetKind,
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

#[derive(Debug, Clone, Default, PartialEq)]
struct ExecutedOutputTargets {
    targets: BTreeMap<RenderTargetId, Vec<OutputRenderRecord>>,
    swapchain_target: Option<RenderTargetId>,
}

pub(crate) fn render_graph_output_records(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputRenderRecord> {
    let executed = execute_output_render_graph(render_graph, render_plan, output_id);
    executed
        .swapchain_target
        .and_then(|target_id| executed.targets.get(&target_id).cloned())
        .unwrap_or_default()
}

pub(crate) fn render_graph_output_records_in_presentation_order(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputRenderRecord> {
    let mut records = render_graph_output_records(render_graph, render_plan, output_id);
    records.reverse();
    records
}

/// Returns output-local surfaces in the front-to-back presentation order expected by backend
/// renderers.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_graph_output_surfaces_in_presentation_order(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> Vec<OutputSurfaceRenderRecord> {
    render_graph_output_records_in_presentation_order(render_graph, render_plan, output_id)
        .into_iter()
        .filter_map(|record| match record {
            OutputRenderRecord::Surface(record) => Some(record),
            OutputRenderRecord::SolidRect(_) | OutputRenderRecord::Backdrop(_) => None,
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
        .filter_map(|record| {
            let (surface_id, instance, kind) = match &record {
                OutputRenderRecord::Surface(record) => (
                    record.surface_id,
                    record.instance,
                    surfaces
                        .get(&record.surface_id)
                        .map(|surface| {
                            present_audit_surface_kind(surface.role, record.instance.scene_role)
                        })
                        .unwrap_or(PresentAuditElementKind::Unknown),
                ),
                OutputRenderRecord::SolidRect(record) => (
                    0,
                    record.instance,
                    if record.instance.scene_role == RenderSceneRole::Compositor {
                        PresentAuditElementKind::Compositor
                    } else {
                        PresentAuditElementKind::SolidRect
                    },
                ),
                OutputRenderRecord::Backdrop(record) => {
                    (0, record.instance, PresentAuditElementKind::Backdrop)
                }
            };
            let visible_rect = instance.visible_rect()?;
            Some(PresentAuditElement {
                surface_id,
                kind,
                x: visible_rect.x,
                y: visible_rect.y,
                width: visible_rect.width,
                height: visible_rect.height,
                z_index: instance.z_index,
                opacity: instance.opacity,
            })
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

fn execute_output_render_graph(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    output_id: OutputId,
) -> ExecutedOutputTargets {
    let Some(execution) = render_graph.outputs.get(&output_id) else {
        return ExecutedOutputTargets::default();
    };
    let Some(output_plan) = render_plan.outputs.get(&output_id) else {
        return ExecutedOutputTargets::default();
    };

    let mut targets = execution
        .targets
        .keys()
        .copied()
        .map(|target_id| (target_id, Vec::new()))
        .collect::<BTreeMap<_, _>>();

    for pass_id in execution.reachable_passes_in_order() {
        let Some(pass) = execution.passes.get(&pass_id) else {
            continue;
        };

        match pass.kind {
            RenderPassKind::Scene => {
                let produced = pass
                    .item_indices()
                    .iter()
                    .filter_map(|item_index| output_plan.items.get(*item_index))
                    .map(output_record_from_plan_item)
                    .collect::<Vec<_>>();
                targets.entry(pass.output_target).or_default().extend(produced);
            }
            RenderPassKind::Composite | RenderPassKind::PostProcess | RenderPassKind::Readback => {
                let produced = pass
                    .input_targets
                    .iter()
                    .filter_map(|target_id| targets.get(target_id))
                    .flat_map(|records| records.iter().cloned())
                    .collect::<Vec<_>>();
                targets.insert(pass.output_target, produced);
            }
        }
    }

    ExecutedOutputTargets {
        swapchain_target: output_swapchain_target(execution, output_id),
        targets,
    }
}

fn output_swapchain_target(
    execution: &OutputExecutionPlan,
    output_id: OutputId,
) -> Option<RenderTargetId> {
    execution.targets.iter().find_map(|(target_id, target_kind)| match target_kind {
        RenderTargetKind::OutputSwapchain(target_output_id) if *target_output_id == output_id => {
            Some(*target_id)
        }
        RenderTargetKind::OutputSwapchain(_)
        | RenderTargetKind::OffscreenColor
        | RenderTargetKind::OffscreenIntermediate => None,
    })
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

fn present_audit_surface_kind(
    role: RenderSurfaceRole,
    scene_role: RenderSceneRole,
) -> PresentAuditElementKind {
    match role {
        RenderSurfaceRole::Window => PresentAuditElementKind::Window,
        RenderSurfaceRole::Popup => PresentAuditElementKind::Popup,
        RenderSurfaceRole::Layer => PresentAuditElementKind::Layer,
        RenderSurfaceRole::Unknown if scene_role == RenderSceneRole::Compositor => {
            PresentAuditElementKind::Compositor
        }
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
        OutputExecutionPlan, OutputRenderPlan, PresentAuditElementKind, RenderColor,
        RenderItemInstance, RenderPassGraph, RenderPassId, RenderPassNode, RenderPlan,
        RenderPlanItem, RenderRect, RenderSceneRole, RenderTargetId, RenderTargetKind,
        SolidRectRenderItem, SurfaceRenderItem,
    };

    use crate::traits::{OutputSnapshot, RenderSurfaceRole, RenderSurfaceSnapshot};

    use super::{
        render_graph_output_present_audit_elements, render_graph_output_records,
        render_graph_output_records_in_presentation_order,
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
                    targets: std::collections::BTreeMap::from([
                        (RenderTargetId(1), RenderTargetKind::OutputSwapchain(OutputId(7))),
                        (RenderTargetId(2), RenderTargetKind::OffscreenColor),
                    ]),
                    passes: std::collections::BTreeMap::from([
                        (
                            RenderPassId(1),
                            RenderPassNode::scene(
                                RenderSceneRole::Desktop,
                                RenderTargetId(2),
                                Vec::new(),
                                vec![0, 1],
                            ),
                        ),
                        (
                            RenderPassId(2),
                            RenderPassNode::composite(
                                RenderSceneRole::Compositor,
                                RenderTargetId(2),
                                RenderTargetId(1),
                                vec![RenderPassId(1)],
                            ),
                        ),
                    ]),
                    ordered_passes: vec![RenderPassId(1), RenderPassId(2)],
                    terminal_passes: vec![RenderPassId(2)],
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
        assert_eq!(
            render_graph_output_records_in_presentation_order(
                &render_graph,
                &render_plan,
                OutputId(7),
            )
            .into_iter()
            .filter_map(|record| match record {
                super::OutputRenderRecord::Surface(record) => Some(record.surface_id),
                super::OutputRenderRecord::SolidRect(_)
                | super::OutputRenderRecord::Backdrop(_) => None,
            })
            .collect::<Vec<_>>(),
            vec![22, 11]
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

    #[test]
    fn audit_records_include_solid_rect_items() {
        let render_plan = RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(1),
                OutputRenderPlan {
                    items: vec![RenderPlanItem::SolidRect(SolidRectRenderItem {
                        color: RenderColor { r: 20, g: 40, b: 60, a: 128 },
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 8, y: 9, width: 50, height: 60 },
                            opacity: 0.75,
                            clip_rect: None,
                            z_index: 5,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    })],
                },
            )]),
        };
        let render_graph = RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                OutputId(1),
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(OutputId(1)),
                    )]),
                    passes: std::collections::BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::scene(
                            RenderSceneRole::Overlay,
                            RenderTargetId(1),
                            Vec::new(),
                            vec![0],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                },
            )]),
        };

        let elements = render_graph_output_present_audit_elements(
            &render_graph,
            &render_plan,
            &HashMap::default(),
            OutputId(1),
        );
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].kind, PresentAuditElementKind::SolidRect);
        assert_eq!(elements[0].surface_id, 0);
        assert_eq!((elements[0].x, elements[0].y), (8, 9));
        assert_eq!((elements[0].width, elements[0].height), (50, 60));
    }

    #[test]
    fn execution_keeps_graph_order_for_non_surface_items() {
        let render_plan = RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(2),
                OutputRenderPlan {
                    items: vec![
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            surface_id: 1,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 0, y: 0, width: 20, height: 20 },
                                opacity: 1.0,
                                clip_rect: None,
                                z_index: 0,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                        RenderPlanItem::SolidRect(SolidRectRenderItem {
                            color: RenderColor { r: 0, g: 0, b: 0, a: 180 },
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 1, y: 2, width: 30, height: 40 },
                                opacity: 0.5,
                                clip_rect: None,
                                z_index: 1,
                                scene_role: RenderSceneRole::Overlay,
                            },
                        }),
                    ],
                },
            )]),
        };
        let render_graph = RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                OutputId(2),
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(OutputId(2)),
                    )]),
                    passes: std::collections::BTreeMap::from([
                        (
                            RenderPassId(1),
                            RenderPassNode::scene(
                                RenderSceneRole::Desktop,
                                RenderTargetId(1),
                                Vec::new(),
                                vec![0],
                            ),
                        ),
                        (
                            RenderPassId(2),
                            RenderPassNode::scene(
                                RenderSceneRole::Overlay,
                                RenderTargetId(1),
                                vec![RenderPassId(1)],
                                vec![1],
                            ),
                        ),
                    ]),
                    ordered_passes: vec![RenderPassId(1), RenderPassId(2)],
                    terminal_passes: vec![RenderPassId(2)],
                },
            )]),
        };

        let records = render_graph_output_records_in_presentation_order(
            &render_graph,
            &render_plan,
            OutputId(2),
        );
        assert!(matches!(records[0], super::OutputRenderRecord::SolidRect(_)));
        assert!(matches!(records[1], super::OutputRenderRecord::Surface(_)));
    }
}
