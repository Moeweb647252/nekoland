use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputFinalTargetPlan, RenderFinalOutputPlan, RenderPassGraph, RenderPassPayload,
    RenderTargetKind,
};

/// Projects the render graph into one explicit present target per output.
pub fn build_render_final_output_plan_system(
    render_graph: Res<'_, RenderPassGraph>,
    mut final_output_plan: ResMut<'_, RenderFinalOutputPlan>,
) {
    let mut outputs = BTreeMap::new();

    for (output_id, execution) in &render_graph.outputs {
        let Some((present_pass_id, present_pass)) =
            execution.ordered_passes.iter().rev().find_map(|pass_id| {
                let pass = execution.passes.get(pass_id)?;
                matches!(
                    execution.targets.get(&pass.output_target),
                    Some(RenderTargetKind::OutputSwapchain(target_output)) if *target_output == *output_id
                )
                .then_some((*pass_id, pass))
            })
        else {
            continue;
        };

        let content_target_id = match &present_pass.payload {
            RenderPassPayload::Scene(_) => present_pass.output_target,
            RenderPassPayload::Composite(config) => config.source_target,
            RenderPassPayload::PostProcess(config) => config.source_target,
            RenderPassPayload::Readback(_) => continue,
        };

        outputs.insert(
            *output_id,
            OutputFinalTargetPlan {
                present_pass_id,
                present_target_id: present_pass.output_target,
                content_target_id,
            },
        );
    }

    final_output_plan.outputs = outputs;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        OutputExecutionPlan, RenderFinalOutputPlan, RenderPassGraph, RenderPassId, RenderPassNode,
        RenderSceneRole, RenderTargetId, RenderTargetKind,
    };

    use super::build_render_final_output_plan_system;

    #[test]
    fn final_output_plan_tracks_present_pass_and_content_target() {
        let mut world = World::default();
        world.insert_resource(RenderPassGraph {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputExecutionPlan {
                    targets: BTreeMap::from([
                        (RenderTargetId(1), RenderTargetKind::OutputSwapchain(OutputId(7))),
                        (RenderTargetId(2), RenderTargetKind::OffscreenColor),
                    ]),
                    passes: BTreeMap::from([
                        (
                            RenderPassId(1),
                            RenderPassNode::scene(
                                RenderSceneRole::Desktop,
                                RenderTargetId(2),
                                Vec::new(),
                                Vec::new(),
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
                        (
                            RenderPassId(3),
                            RenderPassNode::readback(
                                RenderSceneRole::Compositor,
                                RenderTargetId(2),
                                RenderTargetId(2),
                                vec![RenderPassId(2)],
                                Vec::new(),
                            ),
                        ),
                    ]),
                    ordered_passes: vec![RenderPassId(1), RenderPassId(2), RenderPassId(3)],
                    terminal_passes: vec![RenderPassId(2), RenderPassId(3)],
                },
            )]),
        });
        world.init_resource::<RenderFinalOutputPlan>();

        let mut system = IntoSystem::into_system(build_render_final_output_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let plan = world.resource::<RenderFinalOutputPlan>();
        let output = &plan.outputs[&OutputId(7)];
        assert_eq!(output.present_pass_id, RenderPassId(2));
        assert_eq!(output.present_target_id, RenderTargetId(1));
        assert_eq!(output.content_target_id, RenderTargetId(2));
    }
}
