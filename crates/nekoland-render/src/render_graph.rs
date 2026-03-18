use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputExecutionPlan, PendingScreenshotRequests, RenderPassGraph, RenderPassId, RenderPassNode,
    RenderPlan, RenderSceneRole, RenderTargetId, RenderTargetKind,
};

use crate::material::RenderMaterialRequestQueue;

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
    material_requests: Res<'_, RenderMaterialRequestQueue>,
    screenshot_requests: Option<Res<'_, PendingScreenshotRequests>>,
    mut render_graph: ResMut<'_, RenderPassGraph>,
) {
    let mut next_target_id = 1_u64;
    let mut next_pass_id = 1_u64;
    let mut outputs = BTreeMap::new();

    for (output_id, output_plan) in &render_plan.outputs {
        let swapchain_target = RenderTargetId(next_target_id);
        next_target_id = next_target_id.saturating_add(1);
        let output_requests = material_requests.outputs.get(output_id).cloned().unwrap_or_default();
        let output_readbacks = screenshot_requests
            .as_deref()
            .map(|requests| requests.requests_for_output(*output_id))
            .unwrap_or_default();
        let scene_target = if output_requests.is_empty() && output_readbacks.is_empty() {
            swapchain_target
        } else {
            let target = RenderTargetId(next_target_id);
            next_target_id = next_target_id.saturating_add(1);
            target
        };

        let mut execution = OutputExecutionPlan {
            targets: BTreeMap::from([(
                swapchain_target,
                RenderTargetKind::OutputSwapchain(*output_id),
            )]),
            ..Default::default()
        };
        if scene_target != swapchain_target {
            execution.targets.insert(scene_target, RenderTargetKind::OffscreenColor);
        }

        let mut previous_pass = None;
        for scene_role in ROLE_ORDER {
            let item_ids = output_plan
                .ordered_item_ids()
                .iter()
                .copied()
                .filter(|item_id| {
                    output_plan
                        .item(*item_id)
                        .is_some_and(|item| item.instance().scene_role == scene_role)
                })
                .collect::<Vec<_>>();
            if item_ids.is_empty() {
                continue;
            }

            let pass_id = RenderPassId(next_pass_id);
            next_pass_id = next_pass_id.saturating_add(1);
            let dependencies = previous_pass.into_iter().collect::<Vec<_>>();
            execution.passes.insert(
                pass_id,
                RenderPassNode::scene(scene_role, scene_target, dependencies, item_ids),
            );
            execution.ordered_passes.push(pass_id);
            previous_pass = Some(pass_id);
        }

        let mut current_target = scene_target;
        if let Some(mut dependency_pass) = previous_pass {
            for (request_index, request) in output_requests.into_iter().enumerate() {
                let next_target = if request_index % 2 == 0 {
                    let target = RenderTargetId(next_target_id);
                    next_target_id = next_target_id.saturating_add(1);
                    execution.targets.insert(target, RenderTargetKind::OffscreenIntermediate);
                    target
                } else {
                    let target = RenderTargetId(next_target_id);
                    next_target_id = next_target_id.saturating_add(1);
                    execution.targets.insert(target, RenderTargetKind::OffscreenColor);
                    target
                };
                let pass_id = RenderPassId(next_pass_id);
                next_pass_id = next_pass_id.saturating_add(1);
                execution.passes.insert(
                    pass_id,
                    RenderPassNode::post_process(
                        request.scene_role,
                        current_target,
                        next_target,
                        vec![dependency_pass],
                        request.material_id,
                        request.params_id,
                    ),
                );
                execution.ordered_passes.push(pass_id);
                dependency_pass = pass_id;
                current_target = next_target;
            }

            let present_dependency = if current_target != swapchain_target {
                let composite_pass = RenderPassId(next_pass_id);
                next_pass_id = next_pass_id.saturating_add(1);
                execution.passes.insert(
                    composite_pass,
                    RenderPassNode::composite(
                        RenderSceneRole::Compositor,
                        current_target,
                        swapchain_target,
                        vec![dependency_pass],
                    ),
                );
                execution.ordered_passes.push(composite_pass);
                composite_pass
            } else {
                dependency_pass
            };
            execution.terminal_passes.push(present_dependency);

            if !output_readbacks.is_empty() {
                let readback_pass = RenderPassId(next_pass_id);
                next_pass_id = next_pass_id.saturating_add(1);
                execution.passes.insert(
                    readback_pass,
                    RenderPassNode::readback(
                        RenderSceneRole::Compositor,
                        current_target,
                        current_target,
                        vec![present_dependency],
                    ),
                );
                execution.ordered_passes.push(readback_pass);
                execution.terminal_passes.push(readback_pass);
            }
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
        MaterialParamsId, OutputRenderPlan, PendingScreenshotRequests, RenderItemId,
        RenderItemIdentity, RenderItemInstance, RenderMaterialId, RenderPassGraph, RenderPassKind,
        RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole, RenderSourceId,
        SolidRectRenderItem, SurfaceRenderItem,
    };

    use crate::material::{RenderMaterialRequest, RenderMaterialRequestQueue};

    use super::build_render_graph_system;

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[test]
    fn render_graph_builder_emits_one_output_swapchain_chain_per_output() {
        let mut app = App::new();
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<RenderMaterialRequestQueue>()
            .init_resource::<PendingScreenshotRequests>()
            .add_systems(RenderSchedule, build_render_graph_system);

        app.world_mut().resource_mut::<RenderPlan>().outputs = std::collections::BTreeMap::from([
            (
                OutputId(1),
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(11),
                    surface_id: 11,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                })]),
            ),
            (
                OutputId(2),
                OutputRenderPlan::from_items([
                    RenderPlanItem::Surface(SurfaceRenderItem {
                        identity: identity(22),
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
                        identity: identity(23),
                        color: nekoland_ecs::resources::RenderColor { r: 0, g: 0, b: 0, a: 128 },
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 1,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    }),
                ]),
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
        assert_eq!(pass.item_ids(), vec![RenderItemId(11)]);

        let second = &graph.outputs[&OutputId(2)];
        assert_eq!(second.ordered_passes.len(), 2);
        let desktop = &second.passes[&second.ordered_passes[0]];
        let overlay = &second.passes[&second.ordered_passes[1]];
        assert_eq!(desktop.scene_role, RenderSceneRole::Desktop);
        assert_eq!(desktop.item_ids(), vec![RenderItemId(22)]);
        assert_eq!(overlay.scene_role, RenderSceneRole::Overlay);
        assert_eq!(overlay.item_ids(), vec![RenderItemId(23)]);
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
        world.init_resource::<RenderMaterialRequestQueue>();
        world.init_resource::<PendingScreenshotRequests>();

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

    #[test]
    fn render_graph_builder_emits_offscreen_post_process_chain_when_requested() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(1),
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(11),
                    surface_id: 11,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                })]),
            )]),
        });
        world.insert_resource(RenderMaterialRequestQueue {
            outputs: std::collections::BTreeMap::from([(
                OutputId(1),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Desktop,
                    material_id: RenderMaterialId(1),
                    params_id: Some(MaterialParamsId(2)),
                }],
            )]),
        });
        world.init_resource::<RenderPassGraph>();
        world.init_resource::<PendingScreenshotRequests>();

        let mut system = bevy_ecs::system::IntoSystem::into_system(build_render_graph_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let graph = world.resource::<RenderPassGraph>();
        let output = &graph.outputs[&OutputId(1)];
        assert_eq!(output.targets.len(), 3);
        assert_eq!(output.ordered_passes.len(), 3);
        assert_eq!(output.passes[&output.ordered_passes[0]].kind, RenderPassKind::Scene);
        assert_eq!(output.passes[&output.ordered_passes[1]].kind, RenderPassKind::PostProcess);
        assert_eq!(output.passes[&output.ordered_passes[2]].kind, RenderPassKind::Composite);
    }

    #[test]
    fn clear_post_process_requests_resets_all_outputs() {
        let mut world = World::default();
        world.insert_resource(RenderMaterialRequestQueue {
            outputs: std::collections::BTreeMap::from([(
                OutputId(1),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Desktop,
                    material_id: RenderMaterialId(3),
                    params_id: Some(MaterialParamsId(4)),
                }],
            )]),
        });
        world.resource_mut::<RenderMaterialRequestQueue>().outputs.clear();

        assert!(world.resource::<RenderMaterialRequestQueue>().outputs.is_empty());
    }

    #[test]
    fn render_graph_builder_appends_readback_pass_for_pending_screenshot_requests() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                OutputId(1),
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(11),
                    surface_id: 11,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                })]),
            )]),
        });
        world.init_resource::<RenderMaterialRequestQueue>();
        world.init_resource::<RenderPassGraph>();
        let mut screenshot_requests = PendingScreenshotRequests::default();
        let _ = screenshot_requests.request_output(OutputId(1));
        world.insert_resource(screenshot_requests);

        let mut system = bevy_ecs::system::IntoSystem::into_system(build_render_graph_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let graph = world.resource::<RenderPassGraph>();
        let output = &graph.outputs[&OutputId(1)];
        assert_eq!(output.ordered_passes.len(), 3);
        assert_eq!(output.passes[&output.ordered_passes[0]].kind, RenderPassKind::Scene);
        assert_eq!(output.passes[&output.ordered_passes[1]].kind, RenderPassKind::Composite);
        assert_eq!(output.passes[&output.ordered_passes[2]].kind, RenderPassKind::Readback);
        assert_eq!(output.terminal_passes.len(), 2);
    }
}
