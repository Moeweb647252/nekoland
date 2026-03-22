use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputProcessPlan, ProcessInputRef, ProcessShaderKey, ProcessTargetRef, ProcessUniformBlock,
    ProcessUniformValue, ProcessUnit, ProcessUnitId, RenderMaterialFrameState,
    RenderMaterialParamBlock, RenderPassGraph, RenderPassKind, RenderPassPayload,
    RenderProcessPlan,
};

pub fn build_render_process_plan_system(
    render_graph: Res<'_, RenderPassGraph>,
    materials: Res<'_, RenderMaterialFrameState>,
    mut process_plan: ResMut<'_, RenderProcessPlan>,
) {
    let mut next_unit_id = 1_u64;
    let mut outputs = BTreeMap::new();

    for (output_id, execution) in &render_graph.outputs {
        let mut output_process = OutputProcessPlan::default();

        for pass_id in execution.reachable_passes_in_order() {
            let Some(pass) = execution.passes.get(&pass_id) else {
                continue;
            };

            let unit = match (&pass.kind, &pass.payload) {
                (RenderPassKind::Composite, RenderPassPayload::Composite(config)) => {
                    Some(ProcessUnit {
                        shader_key: ProcessShaderKey::BuiltinComposite,
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
                        .map(|descriptor| {
                            ProcessShaderKey::Material(descriptor.pipeline_key.clone())
                        })
                        .unwrap_or(ProcessShaderKey::Passthrough);
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
                        process_regions: config.process_regions.clone(),
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

fn process_uniform_block_from_material_params(
    block: &nekoland_ecs::resources::RenderMaterialParamBlock,
) -> ProcessUniformBlock {
    let values = match block {
        RenderMaterialParamBlock::Empty => std::collections::BTreeMap::default(),
        RenderMaterialParamBlock::Blur(params) => std::collections::BTreeMap::from([(
            "radius".to_owned(),
            ProcessUniformValue::Float(params.radius),
        )]),
        RenderMaterialParamBlock::Shadow(params) => std::collections::BTreeMap::from([
            ("spread".to_owned(), ProcessUniformValue::Float(params.spread)),
            ("offset".to_owned(), ProcessUniformValue::Vec2(params.offset)),
            ("color".to_owned(), ProcessUniformValue::Vec4(params.color)),
        ]),
        RenderMaterialParamBlock::RoundedCorners(params) => std::collections::BTreeMap::from([(
            "radius".to_owned(),
            ProcessUniformValue::Float(params.radius),
        )]),
    };

    ProcessUniformBlock { values }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::system::System;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        MaterialParamsId, OutputExecutionPlan, ProcessRect, ProcessShaderKey,
        RenderMaterialDescriptor, RenderMaterialFrameState, RenderMaterialId, RenderMaterialKind,
        RenderPassGraph, RenderPassId, RenderPassNode, RenderPipelineStage, RenderProcessPlan,
        RenderSceneRole, RenderTargetId, RenderTargetKind,
    };

    use super::build_render_process_plan_system;

    #[test]
    fn process_plan_builder_compiles_composite_and_postprocess_units() {
        let mut world = bevy_ecs::world::World::default();
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
                                vec![ProcessRect { x: 10, y: 20, width: 30, height: 40 }],
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
                    pipeline_key: nekoland_ecs::resources::RenderMaterialPipelineKey::post_process(
                        RenderMaterialKind::BackdropBlur,
                    ),
                    shader_source:
                        nekoland_ecs::resources::RenderMaterialShaderSource::BackdropBlur,
                    bind_group_layout:
                        nekoland_ecs::resources::RenderBindGroupLayoutKey::BlurUniforms,
                    queue_kind:
                        nekoland_ecs::resources::RenderMaterialQueueKind::BackdropPostProcess,
                },
            )]),
            params: BTreeMap::from([(
                MaterialParamsId(3),
                nekoland_ecs::resources::RenderMaterialParamBlock::blur(12.0),
            )]),
        });
        world.init_resource::<RenderProcessPlan>();

        let mut system =
            bevy_ecs::system::IntoSystem::into_system(build_render_process_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let output_plan = &world.resource::<RenderProcessPlan>().outputs[&OutputId(1)];
        assert_eq!(output_plan.ordered_units.len(), 2);
        let post = output_plan.units_for_pass(RenderPassId(1)).next().expect("post unit");
        assert_eq!(
            post.shader_key,
            ProcessShaderKey::Material(nekoland_ecs::resources::RenderMaterialPipelineKey {
                material: RenderMaterialKind::BackdropBlur,
                stage: RenderPipelineStage::PostProcess,
            })
        );
        assert_eq!(post.process_regions, vec![ProcessRect { x: 10, y: 20, width: 30, height: 40 }]);
        let composite = output_plan.units_for_pass(RenderPassId(2)).next().expect("composite");
        assert_eq!(composite.shader_key, ProcessShaderKey::BuiltinComposite);
    }
}
