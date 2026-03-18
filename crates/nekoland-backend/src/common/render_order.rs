use std::collections::{BTreeMap, HashMap};

use nekoland_ecs::components::OutputId;
use nekoland_ecs::resources::{
    CursorRenderSource, OutputExecutionPlan, OutputPresentAudit, PresentAuditElement,
    PresentAuditElementKind, RenderColor, RenderItemInstance, RenderMaterialFrameState,
    RenderPassGraph, RenderPassKind, RenderPassPayload, RenderPlan, RenderPlanItem,
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
pub(crate) struct OutputCursorRenderRecord {
    pub source: CursorRenderSource,
    pub instance: RenderItemInstance,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OutputRenderRecord {
    Surface(OutputSurfaceRenderRecord),
    SolidRect(OutputSolidRectRenderRecord),
    Backdrop(OutputBackdropRenderRecord),
    Cursor(OutputCursorRenderRecord),
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ExecutedOutputTargets {
    targets: BTreeMap<RenderTargetId, Vec<OutputRenderRecord>>,
    swapchain_target: Option<RenderTargetId>,
}

pub(crate) fn render_graph_output_records(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    materials: &RenderMaterialFrameState,
    output_id: OutputId,
) -> Vec<OutputRenderRecord> {
    let executed = execute_output_render_graph(render_graph, render_plan, materials, output_id);
    executed
        .swapchain_target
        .and_then(|target_id| executed.targets.get(&target_id).cloned())
        .unwrap_or_default()
}

pub(crate) fn render_graph_output_records_in_presentation_order(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    materials: &RenderMaterialFrameState,
    output_id: OutputId,
) -> Vec<OutputRenderRecord> {
    let mut records = render_graph_output_records(render_graph, render_plan, materials, output_id);
    records.reverse();
    records
}

/// Returns output-local surfaces in the front-to-back presentation order expected by backend
/// renderers.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_graph_output_surfaces_in_presentation_order(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    materials: &RenderMaterialFrameState,
    output_id: OutputId,
) -> Vec<OutputSurfaceRenderRecord> {
    render_graph_output_records_in_presentation_order(
        render_graph,
        render_plan,
        materials,
        output_id,
    )
    .into_iter()
    .filter_map(|record| match record {
        OutputRenderRecord::Surface(record) => Some(record),
        OutputRenderRecord::SolidRect(_)
        | OutputRenderRecord::Backdrop(_)
        | OutputRenderRecord::Cursor(_) => None,
    })
    .collect::<Vec<_>>()
}

pub(crate) fn render_graph_output_present_audit_elements(
    render_graph: &RenderPassGraph,
    render_plan: &RenderPlan,
    materials: &RenderMaterialFrameState,
    surfaces: &HashMap<u64, RenderSurfaceSnapshot>,
    output_id: OutputId,
) -> Vec<PresentAuditElement> {
    render_graph_output_records(render_graph, render_plan, materials, output_id)
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
                OutputRenderRecord::Cursor(record) => {
                    (0, record.instance, PresentAuditElementKind::Cursor)
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
    materials: &RenderMaterialFrameState,
    surfaces: &HashMap<u64, RenderSurfaceSnapshot>,
) -> BTreeMap<OutputId, OutputPresentAudit> {
    outputs
        .iter()
        .map(|output| {
            let elements = render_graph_output_present_audit_elements(
                render_graph,
                render_plan,
                materials,
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
    materials: &RenderMaterialFrameState,
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
                    .item_ids()
                    .iter()
                    .filter_map(|item_id| output_plan.item(*item_id))
                    .map(output_record_from_plan_item)
                    .collect::<Vec<_>>();
                targets.entry(pass.output_target).or_default().extend(produced);
            }
            RenderPassKind::Composite | RenderPassKind::Readback => {
                let produced = pass
                    .input_targets
                    .iter()
                    .filter_map(|target_id| targets.get(target_id))
                    .flat_map(|records| records.iter().cloned())
                    .collect::<Vec<_>>();
                targets.insert(pass.output_target, produced);
            }
            RenderPassKind::PostProcess => {
                let source_records = pass
                    .input_targets
                    .iter()
                    .filter_map(|target_id| targets.get(target_id))
                    .flat_map(|records| records.iter().cloned())
                    .collect::<Vec<_>>();
                let produced = match &pass.payload {
                    RenderPassPayload::PostProcess(config) => execute_material_records(
                        materials,
                        config.material_id,
                        config.params_id,
                        &source_records,
                    ),
                    RenderPassPayload::Scene(_)
                    | RenderPassPayload::Composite(_)
                    | RenderPassPayload::Readback(_) => source_records,
                };
                targets.insert(pass.output_target, produced);
            }
        }
    }

    ExecutedOutputTargets {
        swapchain_target: output_swapchain_target(execution, output_id),
        targets,
    }
}

fn execute_material_records(
    materials: &RenderMaterialFrameState,
    material_id: nekoland_ecs::resources::RenderMaterialId,
    params_id: Option<nekoland_ecs::resources::MaterialParamsId>,
    source_records: &[OutputRenderRecord],
) -> Vec<OutputRenderRecord> {
    let Some(descriptor) = materials.descriptor(material_id) else {
        return source_records.to_vec();
    };

    match descriptor.pipeline_key.0.as_str() {
        "backdrop_blur" => execute_backdrop_blur_records(materials, params_id, source_records),
        _ => source_records.to_vec(),
    }
}

fn execute_backdrop_blur_records(
    materials: &RenderMaterialFrameState,
    params_id: Option<nekoland_ecs::resources::MaterialParamsId>,
    source_records: &[OutputRenderRecord],
) -> Vec<OutputRenderRecord> {
    let radius = params_id
        .and_then(|params_id| materials.params(params_id))
        .and_then(|params| params.float("radius"))
        .unwrap_or(12.0);
    let overlay_alpha = backdrop_overlay_alpha(radius);
    let mut produced = source_records.to_vec();

    for record in source_records {
        let OutputRenderRecord::Backdrop(record) = record else {
            continue;
        };
        let Some(visible_rect) = record.instance.visible_rect() else {
            continue;
        };
        produced.push(OutputRenderRecord::SolidRect(OutputSolidRectRenderRecord {
            color: RenderColor { r: 255, g: 255, b: 255, a: overlay_alpha },
            instance: RenderItemInstance {
                rect: visible_rect,
                opacity: record.instance.opacity.clamp(0.0, 1.0),
                clip_rect: None,
                z_index: record.instance.z_index.saturating_add(1),
                scene_role: RenderSceneRole::Compositor,
            },
        }));
    }

    produced
}

fn backdrop_overlay_alpha(radius: f32) -> u8 {
    let normalized = (radius.max(0.0) / 24.0).clamp(0.0, 1.0);
    (48.0 + normalized * 96.0).round().clamp(0.0, 255.0) as u8
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
        RenderPlanItem::Cursor(item) => OutputRenderRecord::Cursor(OutputCursorRenderRecord {
            source: item.source.clone(),
            instance: item.instance,
        }),
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
        CursorRenderItem, CursorRenderSource, OutputExecutionPlan, OutputRenderPlan,
        PresentAuditElementKind, RenderColor, RenderItemId, RenderItemIdentity, RenderItemInstance,
        RenderMaterialDescriptor, RenderMaterialFrameState, RenderMaterialId,
        RenderMaterialParamBlock, RenderMaterialParamValue, RenderMaterialPipelineKey,
        RenderPassGraph, RenderPassId, RenderPassNode, RenderPlan, RenderPlanItem, RenderRect,
        RenderSceneRole, RenderSourceId, RenderTargetId, RenderTargetKind, SolidRectRenderItem,
        SurfaceRenderItem,
    };

    use crate::traits::{OutputSnapshot, RenderSurfaceRole, RenderSurfaceSnapshot};

    use super::{
        render_graph_output_present_audit_elements, render_graph_output_records,
        render_graph_output_records_in_presentation_order,
        render_graph_output_surfaces_in_presentation_order, snapshot_present_audit_outputs,
    };

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    fn no_materials() -> RenderMaterialFrameState {
        RenderMaterialFrameState::default()
    }

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
                OutputRenderPlan::from_items([
                    RenderPlanItem::Surface(SurfaceRenderItem {
                        identity: identity(11),
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
                        identity: identity(22),
                        surface_id: 22,
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 50, y: 60, width: 70, height: 80 },
                            opacity: 0.5,
                            clip_rect: None,
                            z_index: 1,
                            scene_role: RenderSceneRole::Desktop,
                        },
                    }),
                ]),
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
                                vec![RenderItemId(11), RenderItemId(22)],
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
            render_graph_output_records(&render_graph, &render_plan, &no_materials(), OutputId(7))
                .into_iter()
                .filter_map(|record| match record {
                    super::OutputRenderRecord::Surface(record) => Some(record.surface_id),
                    super::OutputRenderRecord::SolidRect(_)
                    | super::OutputRenderRecord::Backdrop(_)
                    | super::OutputRenderRecord::Cursor(_) => None,
                })
                .collect::<Vec<_>>(),
            vec![11, 22]
        );
        assert_eq!(
            render_graph_output_records_in_presentation_order(
                &render_graph,
                &render_plan,
                &no_materials(),
                OutputId(7),
            )
            .into_iter()
            .filter_map(|record| match record {
                super::OutputRenderRecord::Surface(record) => Some(record.surface_id),
                super::OutputRenderRecord::SolidRect(_)
                | super::OutputRenderRecord::Backdrop(_)
                | super::OutputRenderRecord::Cursor(_) => None,
            })
            .collect::<Vec<_>>(),
            vec![22, 11]
        );

        let elements = render_graph_output_present_audit_elements(
            &render_graph,
            &render_plan,
            &no_materials(),
            &surfaces,
            OutputId(7),
        );
        assert_eq!(elements[0].kind, PresentAuditElementKind::Window);
        assert_eq!(elements[1].kind, PresentAuditElementKind::Layer);
        assert_eq!(
            render_graph_output_surfaces_in_presentation_order(
                &render_graph,
                &render_plan,
                &no_materials(),
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
            &no_materials(),
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
                OutputRenderPlan::from_items([RenderPlanItem::SolidRect(SolidRectRenderItem {
                    identity: identity(1),
                    color: RenderColor { r: 20, g: 40, b: 60, a: 128 },
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 8, y: 9, width: 50, height: 60 },
                        opacity: 0.75,
                        clip_rect: None,
                        z_index: 5,
                        scene_role: RenderSceneRole::Overlay,
                    },
                })]),
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
                            vec![RenderItemId(1)],
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
            &no_materials(),
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
                OutputRenderPlan::from_items([
                    RenderPlanItem::Surface(SurfaceRenderItem {
                        identity: identity(1),
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
                        identity: identity(2),
                        color: RenderColor { r: 0, g: 0, b: 0, a: 180 },
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 1, y: 2, width: 30, height: 40 },
                            opacity: 0.5,
                            clip_rect: None,
                            z_index: 1,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    }),
                ]),
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
                                vec![RenderItemId(1)],
                            ),
                        ),
                        (
                            RenderPassId(2),
                            RenderPassNode::scene(
                                RenderSceneRole::Overlay,
                                RenderTargetId(1),
                                vec![RenderPassId(1)],
                                vec![RenderItemId(2)],
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
            &no_materials(),
            OutputId(2),
        );
        assert!(matches!(records[0], super::OutputRenderRecord::SolidRect(_)));
        assert!(matches!(records[1], super::OutputRenderRecord::Surface(_)));
    }

    #[test]
    fn audit_records_include_cursor_items() {
        let render_plan = RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(3),
                OutputRenderPlan::from_items([RenderPlanItem::Cursor(CursorRenderItem {
                    identity: identity(30),
                    source: CursorRenderSource::Named { icon_name: "default".to_owned() },
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 12, y: 14, width: 16, height: 24 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: i32::MAX,
                        scene_role: RenderSceneRole::Cursor,
                    },
                })]),
            )]),
        };
        let render_graph = RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                OutputId(3),
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(OutputId(3)),
                    )]),
                    passes: std::collections::BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::scene(
                            RenderSceneRole::Cursor,
                            RenderTargetId(1),
                            Vec::new(),
                            vec![RenderItemId(30)],
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
            &no_materials(),
            &HashMap::default(),
            OutputId(3),
        );
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].kind, PresentAuditElementKind::Cursor);
        assert_eq!(elements[0].surface_id, 0);
        assert_eq!((elements[0].x, elements[0].y), (12, 14));
    }

    #[test]
    fn backdrop_blur_material_adds_compositor_overlay_records() {
        let render_plan = RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(4),
                OutputRenderPlan::from_items([RenderPlanItem::Backdrop(
                    nekoland_ecs::resources::BackdropRenderItem {
                        identity: identity(40),
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 5, y: 6, width: 70, height: 80 },
                            opacity: 0.8,
                            clip_rect: None,
                            z_index: 3,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    },
                )]),
            )]),
        };
        let render_graph = RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                OutputId(4),
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([
                        (RenderTargetId(1), RenderTargetKind::OutputSwapchain(OutputId(4))),
                        (RenderTargetId(2), RenderTargetKind::OffscreenColor),
                        (RenderTargetId(3), RenderTargetKind::OffscreenIntermediate),
                    ]),
                    passes: std::collections::BTreeMap::from([
                        (
                            RenderPassId(1),
                            RenderPassNode::scene(
                                RenderSceneRole::Overlay,
                                RenderTargetId(2),
                                Vec::new(),
                                vec![RenderItemId(40)],
                            ),
                        ),
                        (
                            RenderPassId(2),
                            RenderPassNode::post_process(
                                RenderSceneRole::Compositor,
                                RenderTargetId(2),
                                RenderTargetId(3),
                                vec![RenderPassId(1)],
                                RenderMaterialId(7),
                                Some(nekoland_ecs::resources::MaterialParamsId(9)),
                            ),
                        ),
                        (
                            RenderPassId(3),
                            RenderPassNode::composite(
                                RenderSceneRole::Compositor,
                                RenderTargetId(3),
                                RenderTargetId(1),
                                vec![RenderPassId(2)],
                            ),
                        ),
                    ]),
                    ordered_passes: vec![RenderPassId(1), RenderPassId(2), RenderPassId(3)],
                    terminal_passes: vec![RenderPassId(3)],
                },
            )]),
        };
        let materials = RenderMaterialFrameState {
            descriptors: std::collections::BTreeMap::from([(
                RenderMaterialId(7),
                RenderMaterialDescriptor {
                    debug_name: "backdrop_blur".to_owned(),
                    pipeline_key: RenderMaterialPipelineKey("backdrop_blur".to_owned()),
                },
            )]),
            params: std::collections::BTreeMap::from([(
                nekoland_ecs::resources::MaterialParamsId(9),
                RenderMaterialParamBlock {
                    values: std::collections::BTreeMap::from([(
                        "radius".to_owned(),
                        RenderMaterialParamValue::Float(16.0),
                    )]),
                },
            )]),
        };

        let records =
            render_graph_output_records(&render_graph, &render_plan, &materials, OutputId(4));
        assert_eq!(records.len(), 2);
        assert!(matches!(records[0], super::OutputRenderRecord::Backdrop(_)));
        let super::OutputRenderRecord::SolidRect(overlay) = &records[1] else {
            panic!("expected compositor solid rect overlay");
        };
        assert_eq!(overlay.instance.scene_role, RenderSceneRole::Compositor);
        assert_eq!(overlay.instance.rect, RenderRect { x: 5, y: 6, width: 70, height: 80 });
    }
}
