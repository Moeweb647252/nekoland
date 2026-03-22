use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputReadbackPlan, RenderPassGraph, RenderPassPayload, RenderReadbackPlan,
};

pub fn build_render_readback_plan_system(
    render_graph: Res<'_, RenderPassGraph>,
    mut readback_plan: ResMut<'_, RenderReadbackPlan>,
) {
    let mut outputs = BTreeMap::new();

    for (output_id, execution) in &render_graph.outputs {
        let Some((source_target, request_ids)) =
            execution.ordered_passes.iter().find_map(|pass_id| {
                execution.passes.get(pass_id).and_then(|pass| match &pass.payload {
                    RenderPassPayload::Readback(config) => {
                        Some((config.source_target, config.request_ids.clone()))
                    }
                    RenderPassPayload::Scene(_)
                    | RenderPassPayload::Composite(_)
                    | RenderPassPayload::PostProcess(_) => None,
                })
            })
        else {
            continue;
        };
        if request_ids.is_empty() {
            continue;
        }

        outputs.insert(*output_id, OutputReadbackPlan { source_target, request_ids });
    }

    readback_plan.outputs = outputs;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        OutputExecutionPlan, PendingScreenshotRequests, RenderPassGraph, RenderPassId,
        RenderPassNode, RenderReadbackPlan, RenderSceneRole, RenderTargetId,
    };

    use super::build_render_readback_plan_system;

    #[test]
    fn readback_plan_tracks_pending_requests_per_output() {
        let mut world = World::default();
        let mut requests = PendingScreenshotRequests::default();
        let request_id = requests.request_output(OutputId(7));
        let _ = requests;
        world.insert_resource(RenderPassGraph {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputExecutionPlan {
                    passes: BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::readback(
                            RenderSceneRole::Compositor,
                            RenderTargetId(3),
                            RenderTargetId(3),
                            Vec::new(),
                            vec![request_id],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                    ..Default::default()
                },
            )]),
        });
        world.init_resource::<RenderReadbackPlan>();

        let mut system = IntoSystem::into_system(build_render_readback_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let plan = world.resource::<RenderReadbackPlan>();
        assert_eq!(plan.outputs[&OutputId(7)].source_target, RenderTargetId(3));
        assert_eq!(plan.outputs[&OutputId(7)].request_ids, vec![request_id]);
    }
}
