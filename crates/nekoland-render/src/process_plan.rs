use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputProcessPlan, ProcessInputRef, ProcessRect, ProcessShaderKey, ProcessTargetRef,
    ProcessUniformBlock, ProcessUniformValue, ProcessUnit, ProcessUnitId, RenderMaterialFrameState,
    RenderPassGraph, RenderPassKind, RenderPassPayload, RenderPlan, RenderPlanItem,
    RenderProcessPlan,
};

const BUILTIN_COMPOSITE_SHADER: &str = "builtin.composite";

pub fn build_render_process_plan_system(
    render_plan: Res<'_, RenderPlan>,
    render_graph: Res<'_, RenderPassGraph>,
    materials: Res<'_, RenderMaterialFrameState>,
    mut process_plan: ResMut<'_, RenderProcessPlan>,
) {
    let mut next_unit_id = 1_u64;
    let mut outputs = BTreeMap::new();

    for (output_id, execution) in &render_graph.outputs {
        let output_plan = render_plan.outputs.get(output_id);
        let backdrop_regions = output_plan
            .map(collect_backdrop_regions)
            .unwrap_or_default();

        let mut output_process = OutputProcessPlan::default();

        for pass_id in execution.reachable_passes_in_order() {
            let Some(pass) = execution.passes.get(&pass_id) else {
                continue;
            };

            let unit = match (&pass.kind, &pass.payload) {
                (RenderPassKind::Composite, RenderPassPayload::Composite(config)) => {
                    Some(ProcessUnit {
                        shader_key: ProcessShaderKey(BUILTIN_COMPOSITE_SHADER.to_owned()),
                        input: ProcessInputRef {
                            target_id: config.source_target,
                            sample_rect: None,
                        },
                        output: ProcessTargetRef {
                            target_id: pass.output_target,
                            output_rect: None,
                        },
                        uniforms: ProcessUniformBlock::default(),
                        process_regions: Vec::new(),
                    })
                }
                (RenderPassKind::PostProcess, RenderPassPayload::PostProcess(config)) => {
                    let shader_key = materials
                        .descriptor(config.material_id)
                        .map(|descriptor| ProcessShaderKey(descriptor.pipeline_key.0.clone()))
                        .unwrap_or_else(|| ProcessShaderKey("passthrough".to_owned()));
                    let uniforms = config
                        .params_id
                        .and_then(|params_id| materials.params(params_id))
                        .map(process_uniform_block_from_material_params)
                        .unwrap_or_default();
                    Some(ProcessUnit {
                        shader_key,
                        input: ProcessInputRef {
                            target_id: config.source_target,
                            sample_rect: None,
                        },
                        output: ProcessTargetRef {
                            target_id: pass.output_target,
                            output_rect: None,
                        },
                        uniforms,
                        process_regions: backdrop_regions.clone(),
                    })
                }
                _ => None,
            };

            let Some(unit) = unit else {
                continue;
            };
            let unit_id = ProcessUnitId(next_unit_id);
            next_unit_id = next_unit_id.saturating_add(1);
            output_process.units.insert(unit_id, unit);
            output_process.ordered_units.push(unit_id);
            output_process.pass_units.entry(pass_id).or_default().push(unit_id);
        }

        outputs.insert(*output_id, output_process);
    }

    process_plan.outputs = outputs;
}

fn collect_backdrop_regions(
    output_plan: &nekoland_ecs::resources::OutputRenderPlan,
) -> Vec<ProcessRect> {
    output_plan
        .iter_ordered()
        .filter_map(|item| match item {
            RenderPlanItem::Backdrop(item) => item.instance.visible_rect().map(ProcessRect::from),
            RenderPlanItem::Surface(_)
            | RenderPlanItem::SolidRect(_)
            | RenderPlanItem::Cursor(_) => None,
        })
        .collect::<Vec<_>>()
}

fn process_uniform_block_from_material_params(
    block: &nekoland_ecs::resources::RenderMaterialParamBlock,
) -> ProcessUniformBlock {
    ProcessUniformBlock {
        values: block
            .values
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    nekoland_ecs::resources::RenderMaterialParamValue::Float(value) => {
                        ProcessUniformValue::Float(*value)
                    }
                    nekoland_ecs::resources::RenderMaterialParamValue::Vec2(value) => {
                        ProcessUniformValue::Vec2(*value)
                    }
                    nekoland_ecs::resources::RenderMaterialParamValue::Vec4(value) => {
                        ProcessUniformValue::Vec4(*value)
                    }
                    nekoland_ecs::resources::RenderMaterialParamValue::Rect(value) => {
                        ProcessUniformValue::Rect((*value).into())
                    }
                    nekoland_ecs::resources::RenderMaterialParamValue::Bool(value) => {
                        ProcessUniformValue::Bool(*value)
                    }
                    nekoland_ecs::resources::RenderMaterialParamValue::Uint(value) => {
                        ProcessUniformValue::Uint(*value)
                    }
                };
                (key.clone(), value)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::system::System;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        BackdropRenderItem, MaterialParamsId, OutputExecutionPlan, OutputRenderPlan, ProcessRect,
        ProcessShaderKey, RenderItemId, RenderItemIdentity, RenderItemInstance,
        RenderMaterialDescriptor, RenderMaterialFrameState, RenderMaterialId, RenderPassGraph,
        RenderPassId, RenderPassNode, RenderPlan, RenderPlanItem, RenderProcessPlan, RenderRect,
        RenderSceneRole, RenderSourceId, RenderTargetId, RenderTargetKind,
    };

    use super::build_render_process_plan_system;

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[test]
    fn process_plan_builder_compiles_composite_and_postprocess_units() {
        let mut world = bevy_ecs::world::World::default();
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(1),
                OutputRenderPlan::from_items([RenderPlanItem::Backdrop(BackdropRenderItem {
                    identity: identity(11),
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 10, y: 20, width: 30, height: 40 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 1,
                        scene_role: RenderSceneRole::Overlay,
                    },
                })]),
            )]),
        });
        world.insert_resource(RenderPassGraph {
            outputs: BTreeMap::from([(
                OutputId(1),
                OutputExecutionPlan {
                    targets: BTreeMap::from([
                        (RenderTargetId(1), RenderTargetKind::OutputSwapchain(OutputId(1))),
                        (RenderTargetId(2), RenderTargetKind::OffscreenColor),
                    ]),
                    passes: BTreeMap::from([
                        (
                            RenderPassId(1),
                            RenderPassNode::post_process(
                                RenderSceneRole::Compositor,
                                RenderTargetId(2),
                                RenderTargetId(2),
                                Vec::new(),
                                RenderMaterialId(7),
                                Some(MaterialParamsId(3)),
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
        });
        world.insert_resource(RenderMaterialFrameState {
            descriptors: BTreeMap::from([(
                RenderMaterialId(7),
                RenderMaterialDescriptor {
                    debug_name: "backdrop_blur".to_owned(),
                    pipeline_key: nekoland_ecs::resources::RenderMaterialPipelineKey(
                        "backdrop_blur".to_owned(),
                    ),
                },
            )]),
            params: BTreeMap::from([(
                MaterialParamsId(3),
                nekoland_ecs::resources::RenderMaterialParamBlock {
                    values: BTreeMap::from([(
                        "radius".to_owned(),
                        nekoland_ecs::resources::RenderMaterialParamValue::Float(12.0),
                    )]),
                },
            )]),
        });
        world.init_resource::<RenderProcessPlan>();

        let mut system = bevy_ecs::system::IntoSystem::into_system(build_render_process_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let output_plan = &world.resource::<RenderProcessPlan>().outputs[&OutputId(1)];
        assert_eq!(output_plan.ordered_units.len(), 2);
        let post = output_plan.units_for_pass(RenderPassId(1)).next().expect("post unit");
        assert_eq!(post.shader_key, ProcessShaderKey("backdrop_blur".to_owned()));
        assert_eq!(
            post.process_regions,
            vec![ProcessRect { x: 10, y: 20, width: 30, height: 40 }]
        );
        let composite = output_plan.units_for_pass(RenderPassId(2)).next().expect("composite");
        assert_eq!(composite.shader_key, ProcessShaderKey("builtin.composite".to_owned()));
    }
}
