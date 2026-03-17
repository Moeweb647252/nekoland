use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputExecutionPlan, RenderPassGraph, RenderPassId, RenderPassNode, RenderPlan,
    RenderSceneRole, RenderTargetId, RenderTargetKind,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderGraphBuilder;

const ROLE_ORDER: [RenderSceneRole; 4] = [
    RenderSceneRole::Desktop,
    RenderSceneRole::Compositor,
    RenderSceneRole::Overlay,
    RenderSceneRole::Cursor,
];

/// Projects the current output-local render plan into a backend-neutral execution graph.
pub fn build_render_graph_system(
    render_plan: Res<'_, RenderPlan>,
    mut render_graph: ResMut<'_, RenderPassGraph>,
) {
    let mut next_target_id = 1_u64;
    let mut next_pass_id = 1_u64;
    let mut outputs = BTreeMap::new();

    for (output_id, output_plan) in &render_plan.outputs {
        let swapchain_target = RenderTargetId(next_target_id);
        next_target_id = next_target_id.saturating_add(1);

        let mut execution = OutputExecutionPlan {
            targets: BTreeMap::from([(
                swapchain_target,
                RenderTargetKind::OutputSwapchain(*output_id),
            )]),
            ..Default::default()
        };

        let mut previous_pass = None;
        for scene_role in ROLE_ORDER {
            let item_indices = output_plan
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    (item.instance().scene_role == scene_role).then_some(index)
                })
                .collect::<Vec<_>>();
            if item_indices.is_empty() {
                continue;
            }

            let pass_id = RenderPassId(next_pass_id);
            next_pass_id = next_pass_id.saturating_add(1);
            let dependencies = previous_pass.into_iter().collect::<Vec<_>>();
            execution.passes.insert(
                pass_id,
                RenderPassNode::scene(scene_role, swapchain_target, dependencies, item_indices),
            );
            execution.ordered_passes.push(pass_id);
            previous_pass = Some(pass_id);
        }

        if let Some(terminal_pass) = previous_pass {
            execution.terminal_passes.push(terminal_pass);
        }

        outputs.insert(*output_id, execution);
    }

    render_graph.outputs = outputs;

    debug_assert!(
        render_graph.validate_acyclic(),
        "render pass graph builder must emit an acyclic graph"
    );
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::System;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        OutputRenderPlan, RenderItemInstance, RenderPassGraph, RenderPassKind, RenderPlan,
        RenderPlanItem, RenderRect, RenderSceneRole, SolidRectRenderItem, SurfaceRenderItem,
    };

    use super::build_render_graph_system;

    #[test]
    fn render_graph_builder_emits_one_output_swapchain_chain_per_output() {
        let mut app = App::new();
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .add_systems(RenderSchedule, build_render_graph_system);

        app.world_mut().resource_mut::<RenderPlan>().outputs = std::collections::BTreeMap::from([
            (
                OutputId(1),
                OutputRenderPlan {
                    items: vec![RenderPlanItem::Surface(SurfaceRenderItem {
                        surface_id: 11,
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 0,
                            scene_role: RenderSceneRole::Desktop,
                        },
                    })],
                },
            ),
            (
                OutputId(2),
                OutputRenderPlan {
                    items: vec![
                        RenderPlanItem::Surface(SurfaceRenderItem {
                            surface_id: 22,
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 10, y: 10, width: 80, height: 80 },
                                opacity: 1.0,
                                clip_rect: None,
                                z_index: 0,
                                scene_role: RenderSceneRole::Desktop,
                            },
                        }),
                        RenderPlanItem::SolidRect(SolidRectRenderItem {
                            color: nekoland_ecs::resources::RenderColor {
                                r: 0,
                                g: 0,
                                b: 0,
                                a: 128,
                            },
                            instance: RenderItemInstance {
                                rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                                opacity: 1.0,
                                clip_rect: None,
                                z_index: 1,
                                scene_role: RenderSceneRole::Overlay,
                            },
                        }),
                    ],
                },
            ),
        ]);

        app.world_mut().run_schedule(RenderSchedule);

        let graph = app.world().resource::<RenderPassGraph>();
        assert!(graph.validate_acyclic());
        assert_eq!(graph.outputs.len(), 2);

        let first = &graph.outputs[&OutputId(1)];
        assert_eq!(first.targets.len(), 1);
        assert_eq!(first.ordered_passes.len(), 1);
        let pass = &first.passes[&first.ordered_passes[0]];
        assert_eq!(pass.kind, RenderPassKind::Scene);
        assert_eq!(pass.scene_role, RenderSceneRole::Desktop);
        assert_eq!(pass.item_indices, vec![0]);

        let second = &graph.outputs[&OutputId(2)];
        assert_eq!(second.ordered_passes.len(), 2);
        let desktop = &second.passes[&second.ordered_passes[0]];
        let overlay = &second.passes[&second.ordered_passes[1]];
        assert_eq!(desktop.scene_role, RenderSceneRole::Desktop);
        assert_eq!(desktop.item_indices, vec![0]);
        assert_eq!(overlay.scene_role, RenderSceneRole::Overlay);
        assert_eq!(overlay.item_indices, vec![1]);
        assert_eq!(overlay.dependencies, vec![second.ordered_passes[0]]);
        assert_eq!(second.terminal_passes, vec![second.ordered_passes[1]]);
    }

    #[test]
    fn render_graph_builder_skips_empty_outputs_but_keeps_swapchain_targets() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([(OutputId(3), OutputRenderPlan::default())]),
        });
        world.init_resource::<RenderPassGraph>();

        let mut system = bevy_ecs::system::IntoSystem::into_system(build_render_graph_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let graph = world.resource::<RenderPassGraph>();
        let output = &graph.outputs[&OutputId(3)];
        assert_eq!(output.targets.len(), 1);
        assert!(output.passes.is_empty());
        assert!(output.ordered_passes.is_empty());
        assert!(output.terminal_passes.is_empty());
    }
}
