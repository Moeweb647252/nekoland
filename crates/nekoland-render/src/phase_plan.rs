use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{
    OutputPhasePlan, PostProcessPhaseItem, ReadbackPhaseItem, RenderPhasePlan, RenderPlan,
    RenderSceneRole, ScenePhaseItem, ShellRenderInput,
};

use crate::material::RenderMaterialRequestQueue;

const ROLE_ORDER: [RenderSceneRole; 4] = [
    RenderSceneRole::Desktop,
    RenderSceneRole::Compositor,
    RenderSceneRole::Overlay,
    RenderSceneRole::Cursor,
];

/// Builds generic output-local phase lists ahead of render-graph compilation.
pub fn build_render_phase_plan_system(
    render_plan: Res<'_, RenderPlan>,
    material_requests: Res<'_, RenderMaterialRequestQueue>,
    shell_render_input: Res<'_, ShellRenderInput>,
    mut phase_plan: ResMut<'_, RenderPhasePlan>,
) {
    let mut outputs = BTreeMap::new();

    for (output_id, output_plan) in &render_plan.outputs {
        let mut scene_passes = Vec::new();
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
            scene_passes.push(ScenePhaseItem { scene_role, item_ids });
        }

        let post_process_passes = material_requests
            .outputs
            .get(output_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|request| PostProcessPhaseItem {
                scene_role: request.scene_role,
                material_id: request.material_id,
                params_id: request.params_id,
                process_regions: request.process_regions,
            })
            .collect::<Vec<_>>();

        let readback =
            Some(shell_render_input.pending_screenshot_requests.requests_for_output(*output_id))
                .filter(|requests| !requests.is_empty())
                .map(|requests| ReadbackPhaseItem {
                    request_ids: requests.into_iter().map(|request| request.id).collect(),
                });

        outputs.insert(*output_id, OutputPhasePlan { scene_passes, post_process_passes, readback });
    }

    phase_plan.outputs = outputs;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        MaterialParamsId, OutputRenderPlan, PendingScreenshotRequests, RenderItemId,
        RenderItemIdentity, RenderItemInstance, RenderMaterialId, RenderPhasePlan, RenderPlan,
        RenderPlanItem, RenderRect, RenderSceneRole, RenderSourceId, ShellRenderInput,
        QuadContent, QuadRenderItem, SurfaceRenderItem,
    };

    use crate::material::{RenderMaterialRequest, RenderMaterialRequestQueue};

    use super::build_render_phase_plan_system;

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[test]
    fn phase_plan_groups_scene_postprocess_and_readback_per_output() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(1),
                OutputRenderPlan::from_items([
                    RenderPlanItem::Surface(SurfaceRenderItem {
                        identity: identity(11),
                        surface_id: 11,
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 0,
                            scene_role: RenderSceneRole::Desktop,
                        },
                    }),
                    RenderPlanItem::Quad(QuadRenderItem {
                        identity: identity(12),
                        content: QuadContent::SolidColor {
                            color: nekoland_ecs::resources::RenderColor {
                                r: 0,
                                g: 0,
                                b: 0,
                                a: 255,
                            },
                        },
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 100, height: 30 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 1,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    }),
                ]),
            )]),
        });
        world.insert_resource(RenderMaterialRequestQueue {
            outputs: BTreeMap::from([(
                OutputId(1),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Desktop,
                    material_id: RenderMaterialId(3),
                    params_id: Some(MaterialParamsId(4)),
                    process_regions: Vec::new(),
                }],
            )]),
        });
        let mut screenshot_requests = PendingScreenshotRequests::default();
        let request_id = screenshot_requests.request_output(OutputId(1));
        world.insert_resource(ShellRenderInput {
            pending_screenshot_requests: screenshot_requests,
            ..Default::default()
        });
        world.init_resource::<RenderPhasePlan>();

        let mut system = IntoSystem::into_system(build_render_phase_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let plan = world.resource::<RenderPhasePlan>();
        let output = &plan.outputs[&OutputId(1)];
        assert_eq!(output.scene_passes.len(), 2);
        assert_eq!(output.scene_passes[0].scene_role, RenderSceneRole::Desktop);
        assert_eq!(output.scene_passes[0].item_ids, vec![RenderItemId(11)]);
        assert_eq!(output.scene_passes[1].scene_role, RenderSceneRole::Overlay);
        assert_eq!(output.scene_passes[1].item_ids, vec![RenderItemId(12)]);
        assert_eq!(output.post_process_passes.len(), 1);
        assert_eq!(output.post_process_passes[0].material_id, RenderMaterialId(3));
        assert_eq!(
            output.readback.as_ref().map(|item| item.request_ids.clone()),
            Some(vec![request_id])
        );
    }
}
