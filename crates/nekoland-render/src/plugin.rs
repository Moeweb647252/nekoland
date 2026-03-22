use bevy_app::{App, SubApp};
use bevy_ecs::prelude::{Res, ResMut};
use bevy_ecs::schedule::{InternedScheduleLabel, IntoScheduleConfigs, ScheduleLabel, SystemSet};
use bevy_ecs::world::World;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{PreRenderSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    CompiledOutputFrames, CompositorClock, CompositorSceneState, CursorSceneSnapshot, DamageState,
    FramePacingState, OutputDamageRegions, PreparedGpuResources, PreparedSceneResources,
    RenderFinalOutputPlan, RenderMaterialFrameState,
    RenderPassGraph, RenderPhasePlan, RenderPlan, RenderProcessPlan, RenderReadbackPlan,
    RenderTargetAllocationPlan, ShellRenderInput, SurfaceBufferAttachmentSnapshot,
    SurfaceContentVersionSnapshot, SurfaceTextureBridgePlan, WaylandIngress,
};

use crate::{
    animation, compositor_render, cursor, damage_tracker, effects, final_output_plan,
    frame_callback, material, output_overlay, phase_plan, pipeline_cache, prepare_resources,
    presentation_feedback, process_plan, readback_plan, render_graph, scene_process, scene_source,
    screenshot,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderPlugin;

#[derive(Debug, Default, Clone, Copy)]
pub struct RenderSubAppPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderPrepareSystems;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderQueueSystems;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderExecuteSystems;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderCleanupSystems;

impl NekolandPlugin for RenderPlugin {
    /// Register render-stage resources plus the strictly ordered render pipeline
    /// that keeps compositor-internal rendering separate from user-facing visual state.
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<RenderPhasePlan>()
            .init_resource::<RenderProcessPlan>()
            .init_resource::<material::RenderMaterialRegistry>()
            .init_resource::<material::RenderMaterialParamsStore>()
            .init_resource::<material::RenderMaterialRequestQueue>()
            .init_resource::<RenderMaterialFrameState>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<output_overlay::OutputOverlaySceneSyncState>()
            .init_resource::<scene_source::RenderSceneContributionQueue>()
            .init_resource::<scene_source::RenderSceneIdentityRegistry>()
            .init_resource::<compositor_render::DesktopSurfaceOrderSnapshot>()
            .init_resource::<animation::AnimationTimelineStore>()
            .init_resource::<CursorSceneSnapshot>()
            .init_resource::<cursor::CursorThemeGeometryCache>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<scene_process::AppearanceSnapshot>()
            .init_resource::<scene_process::ProjectionSnapshot>()
            .init_resource::<pipeline_cache::RenderPipelineCacheState>()
            .init_resource::<PreparedSceneResources>()
            .init_resource::<PreparedGpuResources>()
            .init_resource::<RenderTargetAllocationPlan>()
            .init_resource::<SurfaceTextureBridgePlan>()
            .init_resource::<RenderFinalOutputPlan>()
            .init_resource::<RenderReadbackPlan>()
            .add_systems(
                PreRenderSchedule,
                (
                    animation::advance_animation_timelines_system,
                    scene_process::prune_stale_compositor_animation_tracks_system,
                )
                    .chain(),
            );

        effects::install_main_render_features(app);
    }
}

impl NekolandPlugin for RenderSubAppPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<RenderPhasePlan>()
            .init_resource::<RenderProcessPlan>()
            .init_resource::<material::RenderMaterialRegistry>()
            .init_resource::<material::RenderMaterialParamsStore>()
            .init_resource::<material::RenderMaterialRequestQueue>()
            .init_resource::<RenderMaterialFrameState>()
            .init_resource::<CompiledOutputFrames>()
            .init_resource::<DamageState>()
            .init_resource::<FramePacingState>()
            .init_resource::<OutputDamageRegions>()
            .init_resource::<SurfaceContentVersionSnapshot>()
            .init_resource::<compositor_render::RenderViewSnapshot>()
            .init_resource::<compositor_render::DesktopSurfaceOrderSnapshot>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<CursorSceneSnapshot>()
            .init_resource::<CompositorClock>()
            .init_resource::<ShellRenderInput>()
            .init_resource::<output_overlay::OutputOverlaySceneSyncState>()
            .init_resource::<scene_source::RenderSceneContributionQueue>()
            .init_resource::<scene_source::RenderSceneIdentityRegistry>()
            .init_resource::<scene_process::AppearanceSnapshot>()
            .init_resource::<scene_process::ProjectionSnapshot>()
            .init_resource::<cursor::CursorThemeGeometryCache>()
            .init_resource::<pipeline_cache::RenderPipelineCacheState>()
            .init_resource::<SurfaceBufferAttachmentSnapshot>()
            .init_resource::<PreparedSceneResources>()
            .init_resource::<PreparedGpuResources>()
            .init_resource::<RenderTargetAllocationPlan>()
            .init_resource::<SurfaceTextureBridgePlan>()
            .init_resource::<RenderFinalOutputPlan>()
            .init_resource::<RenderReadbackPlan>()
            .configure_sets(
                RenderSchedule,
                (
                    RenderPrepareSystems,
                    RenderQueueSystems.after(RenderPrepareSystems),
                    RenderExecuteSystems.after(RenderQueueSystems),
                    RenderCleanupSystems.after(RenderExecuteSystems),
                ),
            )
            .add_systems(
                RenderSchedule,
                (
                    material::clear_material_requests_system,
                    output_overlay::sync_output_overlay_scene_state_system,
                    scene_source::clear_scene_contributions_system,
                    compositor_render::emit_desktop_scene_contributions_from_snapshot_system,
                    scene_source::emit_compositor_scene_contributions_system,
                    cursor::cursor_scene_snapshot_system,
                    cursor::emit_cursor_scene_contributions_system,
                    compositor_render::assemble_render_plan_from_snapshot_system,
                    prepare_resources::build_surface_texture_bridge_plan_system,
                    prepare_resources::build_prepared_scene_resources_system,
                    material::project_material_frame_state_system,
                )
                    .chain()
                    .in_set(RenderPrepareSystems),
            )
            .add_systems(
                RenderSchedule,
                (
                    phase_plan::build_render_phase_plan_system,
                    render_graph::build_render_graph_system,
                    final_output_plan::build_render_final_output_plan_system,
                    readback_plan::build_render_readback_plan_system,
                    process_plan::build_render_process_plan_system,
                    prepare_resources::build_render_target_allocation_plan_system,
                    prepare_resources::build_prepared_gpu_resources_system,
                    pipeline_cache::build_render_pipeline_cache_state_system,
                    pipeline_cache::build_process_pipeline_cache_state_system,
                )
                    .chain()
                    .in_set(RenderQueueSystems),
            )
            .add_systems(
                RenderSchedule,
                (damage_tracker::damage_tracking_system, sync_compiled_output_frames_system)
                    .chain()
                    .in_set(RenderExecuteSystems),
            )
            .add_systems(
                RenderSchedule,
                (
                    frame_callback::frame_callback_system,
                    presentation_feedback::presentation_feedback_system,
                    screenshot::screenshot_system,
                )
                    .chain()
                    .in_set(RenderCleanupSystems),
            );

        effects::install_render_subapp_features(app);
    }
}

pub fn configure_render_subapp(sub_app: &mut SubApp) {
    sub_app.update_schedule = Some(RenderSchedule.intern());
    sub_app.set_extract(extract_render_subapp_inputs);
}

fn extract_render_subapp_inputs(main_world: &mut World, render_world: &mut World) {
    clone_resource_into::<material::RenderMaterialRegistry>(main_world, render_world);
    clone_resource_into::<material::RenderMaterialParamsStore>(main_world, render_world);
    clone_resource_into::<material::RenderMaterialRequestQueue>(main_world, render_world);
    clone_resource_into::<ShellRenderInput>(main_world, render_world);
    clone_resource_into::<OutputDamageRegions>(main_world, render_world);
    clone_resource_into::<CompositorSceneState>(main_world, render_world);
    clone_resource_into::<CompositorClock>(main_world, render_world);
    clone_resource_into::<effects::blur::BlurEffectConfig>(main_world, render_world);
    clone_resource_into::<effects::shadow::ShadowEffectConfig>(main_world, render_world);
    clone_resource_into::<effects::rounded_corners::RoundedCornerEffectConfig>(
        main_world,
        render_world,
    );
    sync_render_platform_inputs_from_wayland_ingress(main_world, render_world);
    scene_process::extract_scene_process_snapshots(main_world, render_world);
    extract_render_view_snapshot(main_world, render_world);
    extract_desktop_surface_order_snapshot(main_world, render_world);
    extract_surface_content_versions_snapshot(main_world, render_world);
    extract_surface_buffer_attachment_snapshot(main_world, render_world);
}

pub fn sync_render_subapp_back(
    main_world: &mut World,
    render_world: &mut World,
    _schedule: Option<InternedScheduleLabel>,
) {
    clone_resource_into::<RenderMaterialFrameState>(render_world, main_world);
    clone_resource_into::<RenderPassGraph>(render_world, main_world);
    clone_resource_into::<RenderProcessPlan>(render_world, main_world);
    clone_resource_into::<RenderFinalOutputPlan>(render_world, main_world);
    clone_resource_into::<RenderReadbackPlan>(render_world, main_world);
    clone_resource_into::<CompiledOutputFrames>(render_world, main_world);
    clone_resource_into::<DamageState>(render_world, main_world);
    clone_resource_into::<FramePacingState>(render_world, main_world);
    clone_resource_into::<OutputDamageRegions>(render_world, main_world);
    clone_resource_into::<pipeline_cache::RenderPipelineCacheState>(render_world, main_world);
    clone_resource_into::<PreparedSceneResources>(render_world, main_world);
    clone_resource_into::<PreparedGpuResources>(render_world, main_world);
    clone_resource_into::<RenderTargetAllocationPlan>(render_world, main_world);
    clone_resource_into::<SurfaceTextureBridgePlan>(render_world, main_world);
}

fn clone_resource_into<R>(source: &World, dest: &mut World)
where
    R: bevy_ecs::prelude::Resource + Clone,
{
    if let Some(resource) = source.get_resource::<R>() {
        dest.insert_resource(resource.clone());
    }
}

fn sync_render_platform_inputs_from_wayland_ingress(main_world: &World, render_world: &mut World) {
    let Some(wayland_ingress) = main_world.get_resource::<WaylandIngress>() else {
        return;
    };

    render_world.insert_resource(wayland_ingress.surface_snapshots.clone());
}

fn sync_compiled_output_frames_system(
    output_damage_regions: Res<'_, OutputDamageRegions>,
    prepared_scene: Res<'_, PreparedSceneResources>,
    prepared_gpu: Res<'_, PreparedGpuResources>,
    materials: Res<'_, RenderMaterialFrameState>,
    render_graph: Res<'_, RenderPassGraph>,
    render_plan: Res<'_, RenderPlan>,
    process_plan: Res<'_, RenderProcessPlan>,
    final_output_plan: Res<'_, RenderFinalOutputPlan>,
    readback_plan: Res<'_, RenderReadbackPlan>,
    render_target_allocation: Res<'_, RenderTargetAllocationPlan>,
    surface_texture_bridge: Res<'_, SurfaceTextureBridgePlan>,
    mut compiled: ResMut<'_, CompiledOutputFrames>,
) {
    let outputs = render_plan
        .outputs
        .iter()
        .map(|(output_id, output_render_plan)| {
            (
                *output_id,
                nekoland_ecs::resources::CompiledOutputFrame {
                    render_plan: output_render_plan.clone(),
                    prepared_scene: prepared_scene
                        .outputs
                        .get(output_id)
                        .cloned()
                        .unwrap_or_default(),
                    execution_plan: render_graph
                        .outputs
                        .get(output_id)
                        .cloned()
                        .unwrap_or_default(),
                    process_plan: process_plan.outputs.get(output_id).cloned().unwrap_or_default(),
                    final_output: final_output_plan.outputs.get(output_id).cloned(),
                    readback: readback_plan.outputs.get(output_id).cloned(),
                    target_allocation: render_target_allocation.outputs.get(output_id).cloned(),
                    gpu_prep: prepared_gpu.outputs.get(output_id).cloned(),
                    damage_regions: output_damage_regions
                        .regions
                        .get(output_id)
                        .cloned()
                        .unwrap_or_default(),
                },
            )
        })
        .collect();

    *compiled = CompiledOutputFrames {
        outputs,
        output_damage_regions: output_damage_regions.clone(),
        prepared_scene: prepared_scene.clone(),
        materials: materials.clone(),
        render_graph: render_graph.clone(),
        render_plan: render_plan.clone(),
        process_plan: process_plan.clone(),
        final_output_plan: final_output_plan.clone(),
        readback_plan: readback_plan.clone(),
        render_target_allocation: render_target_allocation.clone(),
        surface_texture_bridge: surface_texture_bridge.clone(),
        prepared_gpu: prepared_gpu.clone(),
    };
}

fn extract_render_view_snapshot(main_world: &mut World, render_world: &mut World) {
    let views = main_world
        .get_resource::<WaylandIngress>()
        .map(|ingress| {
            ingress
                .output_snapshots
                .outputs
                .iter()
                .map(|output| compositor_render::RenderViewState {
                    output_id: output.output_id,
                    x: output.x,
                    y: output.y,
                    width: output.width,
                    height: output.height,
                    scale: output.scale,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    render_world.insert_resource(compositor_render::RenderViewSnapshot { views });
}

fn extract_desktop_surface_order_snapshot(main_world: &mut World, render_world: &mut World) {
    let mut layers = main_world.query_filtered::<
        nekoland_ecs::views::LayerRenderRuntime,
        bevy_ecs::query::With<nekoland_ecs::components::LayerShellSurface>,
    >();
    let mut windows = main_world.query_filtered::<(
        bevy_ecs::entity::Entity,
        nekoland_ecs::views::WindowRenderRuntime,
    ), bevy_ecs::query::With<nekoland_ecs::components::XdgWindow>>(
    );
    let mut popups = main_world.query_filtered::<
        nekoland_ecs::views::PopupRenderRuntime,
        bevy_ecs::query::With<nekoland_ecs::components::XdgPopup>,
    >();
    let mut workspaces =
        main_world.query::<(bevy_ecs::entity::Entity, nekoland_ecs::views::WorkspaceRuntime)>();

    let stacking = main_world
        .get_resource::<nekoland_ecs::resources::WindowStackingState>()
        .cloned()
        .unwrap_or_default();
    let surface_presentation = main_world
        .get_resource::<ShellRenderInput>()
        .map(|mailbox| mailbox.surface_presentation.clone());
    let surface_presentation = surface_presentation.as_ref();
    let live_outputs = main_world
        .get_resource::<WaylandIngress>()
        .map(|ingress| {
            ingress
                .output_snapshots
                .outputs
                .iter()
                .map(|output| output.output_id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let workspace_ids_by_entity = workspaces
        .iter(main_world)
        .map(|(entity, workspace)| (entity, workspace.id().0))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut ordered = live_outputs
        .iter()
        .copied()
        .map(|output_id| (output_id, Vec::new()))
        .collect::<std::collections::BTreeMap<_, Vec<u64>>>();

    let background_windows = windows
        .iter(main_world)
        .filter_map(|(_, window)| {
            let state = surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&window.surface_id()));
            let visible = state.map_or_else(
                || {
                    nekoland_ecs::presentation_logic::output_background_window_visible(
                        *window.mode,
                        window.background.is_some(),
                        *window.role,
                    )
                },
                |state| {
                    state.visible
                        && state.role
                            == nekoland_ecs::resources::SurfacePresentationRole::OutputBackground
                },
            );
            if !visible {
                return None;
            }
            let output_id = state
                .and_then(|state| state.target_output)
                .or_else(|| window.background.as_ref().map(|background| background.output))?;
            Some((output_id, window.surface_id()))
        })
        .fold(std::collections::BTreeMap::new(), |mut backgrounds, (output_id, candidate)| {
            backgrounds
                .entry(output_id)
                .and_modify(|current: &mut u64| {
                    if candidate > *current {
                        *current = candidate;
                    }
                })
                .or_insert(candidate);
            backgrounds
        })
        .into_values()
        .collect::<Vec<_>>();

    let visible_windows = windows
        .iter(main_world)
        .filter_map(|(entity, window)| {
            let state = surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&window.surface_id()));
            let visible = state.map_or_else(
                || {
                    nekoland_ecs::presentation_logic::managed_window_visible(
                        *window.mode,
                        window.viewport_visibility.visible,
                        *window.role,
                    )
                },
                |state| {
                    state.visible
                        && state.role == nekoland_ecs::resources::SurfacePresentationRole::Window
                },
            );
            visible.then_some((
                entity,
                window.surface_id(),
                window
                    .child_of
                    .and_then(|child_of| workspace_ids_by_entity.get(&child_of.parent()).copied())
                    .unwrap_or(nekoland_ecs::resources::UNASSIGNED_WORKSPACE_STACK_ID),
            ))
        })
        .collect::<Vec<_>>();
    let active_window_entities = visible_windows
        .iter()
        .map(|(entity, ..)| *entity)
        .collect::<std::collections::BTreeSet<_>>();
    let ordered_window_surfaces = stacking.ordered_surfaces(
        visible_windows.iter().map(|(_, surface_id, workspace_id)| (*workspace_id, *surface_id)),
    );
    let background_layer_surfaces = layers
        .iter(main_world)
        .filter(|layer| {
            surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&layer.surface_id()))
                .map_or_else(
                    || {
                        layer.buffer.attached
                            && nekoland_ecs::presentation_logic::is_background_band_layer(
                                layer.layer_surface.layer,
                            )
                    },
                    |state| {
                        state.visible
                            && state.role == nekoland_ecs::resources::SurfacePresentationRole::Layer
                            && nekoland_ecs::presentation_logic::is_background_band_layer(
                                layer.layer_surface.layer,
                            )
                    },
                )
        })
        .map(|layer| layer.surface_id())
        .collect::<Vec<_>>();
    let popup_surfaces = popups
        .iter(main_world)
        .filter(|popup| {
            surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&popup.surface_id()))
                .map_or_else(
                    || {
                        nekoland_ecs::presentation_logic::popup_visible(
                            popup.buffer.attached,
                            active_window_entities.contains(&popup.child_of.parent()),
                        )
                    },
                    |state| {
                        state.visible
                            && state.role == nekoland_ecs::resources::SurfacePresentationRole::Popup
                    },
                )
        })
        .map(|popup| popup.surface_id())
        .collect::<Vec<_>>();
    let foreground_layer_surfaces = layers
        .iter(main_world)
        .filter(|layer| {
            surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&layer.surface_id()))
                .map_or_else(
                    || {
                        layer.buffer.attached
                            && nekoland_ecs::presentation_logic::is_foreground_band_layer(
                                layer.layer_surface.layer,
                            )
                    },
                    |state| {
                        state.visible
                            && state.role == nekoland_ecs::resources::SurfacePresentationRole::Layer
                            && nekoland_ecs::presentation_logic::is_foreground_band_layer(
                                layer.layer_surface.layer,
                            )
                    },
                )
        })
        .map(|layer| layer.surface_id())
        .collect::<Vec<_>>();

    let elements = background_windows
        .into_iter()
        .chain(background_layer_surfaces)
        .chain(ordered_window_surfaces)
        .chain(popup_surfaces)
        .chain(foreground_layer_surfaces)
        .collect::<Vec<_>>();

    for surface_id in elements {
        let Some(state) =
            surface_presentation.and_then(|snapshot| snapshot.surfaces.get(&surface_id))
        else {
            continue;
        };
        if !state.visible {
            continue;
        }

        let target_outputs = if let Some(target_output_id) = state.target_output {
            vec![target_output_id]
        } else {
            live_outputs.clone()
        };

        for output_id in target_outputs {
            ordered.entry(output_id).or_default().push(surface_id);
        }
    }

    render_world
        .insert_resource(compositor_render::DesktopSurfaceOrderSnapshot { outputs: ordered });
}

fn extract_surface_content_versions_snapshot(main_world: &mut World, render_world: &mut World) {
    let versions = main_world
        .get_resource::<WaylandIngress>()
        .map(|ingress| ingress.surface_snapshots.clone())
        .map(|surfaces| {
            surfaces
                .surfaces
                .iter()
                .map(|(surface_id, surface)| (*surface_id, surface.content_version))
                .collect()
        })
        .unwrap_or_default();
    render_world.insert_resource(SurfaceContentVersionSnapshot { versions });
}

fn extract_surface_buffer_attachment_snapshot(main_world: &mut World, render_world: &mut World) {
    let snapshot = main_world
        .get_resource::<WaylandIngress>()
        .map(|ingress| ingress.surface_snapshots.clone())
        .map(|surfaces| {
            surfaces
                .surfaces
                .iter()
                .map(|(surface_id, surface)| {
                    (
                        *surface_id,
                        nekoland_ecs::resources::SurfaceBufferAttachmentState {
                            attached: surface.attached,
                            scale: surface.scale,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    render_world.insert_resource(SurfaceBufferAttachmentSnapshot { surfaces: snapshot });
}

#[cfg(test)]
mod tests {
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{OutputId, SurfaceContentVersion, WlSurfaceHandle, XdgWindow};
    use nekoland_ecs::resources::{
        CompiledOutputFrames, CompositorClock, CursorImageSnapshot, GlobalPointerPosition,
        OutputDamageRegions, OutputExecutionPlan, OutputGeometrySnapshot, OutputOverlayState,
        OutputRenderPlan, OutputSnapshotState, PlatformSurfaceSnapshotState, PreparedGpuResources,
        PreparedSceneResources, RenderFinalOutputPlan, RenderItemId, RenderItemIdentity,
        RenderItemInstance, RenderMaterialFrameState, RenderPassGraph, RenderPassId,
        RenderPassNode, RenderPlan, RenderPlanItem, RenderProcessPlan, RenderReadbackPlan,
        RenderRect, RenderSceneRole, RenderSourceId, RenderTargetAllocationPlan, RenderTargetId,
        RenderTargetKind, ShellRenderInput, SurfaceBufferAttachmentSnapshot,
        SurfaceContentVersionSnapshot, SurfacePresentationRole, SurfacePresentationSnapshot,
        SurfacePresentationState, SurfaceRenderItem, SurfaceTextureBridgePlan, WaylandIngress,
    };

    use crate::animation::{
        AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
        AnimationTrack, AnimationValue, advance_animation_timelines_system,
    };
    use crate::compositor_render::RenderViewSnapshot;
    use crate::scene_process::{AppearanceSnapshot, ProjectionSnapshot};
    use crate::scene_source::{RenderInstanceKey, RenderSourceKey};

    use super::{extract_render_subapp_inputs, sync_compiled_output_frames_system};

    #[test]
    fn render_subapp_extract_syncs_shell_owned_inputs_from_shell_render_mailbox() {
        let mut main_world = bevy_ecs::world::World::default();
        let mut pending_screenshot_requests =
            nekoland_ecs::resources::PendingScreenshotRequests::default();
        let request_id = pending_screenshot_requests.request_output(OutputId(3));
        main_world.insert_resource(ShellRenderInput {
            pointer: GlobalPointerPosition { x: 33.0, y: 44.0 },
            cursor_image: CursorImageSnapshot::Named { icon_name: "default".to_owned() },
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: std::collections::BTreeMap::from([(
                    77,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(OutputId(3)),
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 1,
                            y: 2,
                            width: 100,
                            height: 200,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                )]),
            },
            output_overlays: OutputOverlayState::default(),
            pending_screenshot_requests,
        });

        let mut render_world = bevy_ecs::world::World::default();
        extract_render_subapp_inputs(&mut main_world, &mut render_world);

        assert_eq!(render_world.resource::<ShellRenderInput>().pointer.x, 33.0);
        assert_eq!(render_world.resource::<ShellRenderInput>().pointer.y, 44.0);
        assert_eq!(
            render_world.resource::<ShellRenderInput>().cursor_image,
            CursorImageSnapshot::Named { icon_name: "default".to_owned() }
        );
        assert!(
            render_world
                .resource::<ShellRenderInput>()
                .surface_presentation
                .surfaces
                .contains_key(&77)
        );
        let requests = &render_world.resource::<ShellRenderInput>().pending_screenshot_requests;
        assert_eq!(requests.requests.len(), 1);
        assert_eq!(requests.requests[0].id, request_id);
        assert_eq!(requests.requests[0].output_id, OutputId(3));
    }

    #[test]
    fn render_subapp_extract_builds_view_and_surface_snapshots_from_mailboxes() {
        let mut main_world = bevy_ecs::world::World::default();
        main_world.insert_resource(nekoland_ecs::resources::WindowStackingState {
            workspaces: std::collections::BTreeMap::from([(
                nekoland_ecs::resources::UNASSIGNED_WORKSPACE_STACK_ID,
                vec![42],
            )]),
        });
        main_world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![OutputGeometrySnapshot {
                    output_id: OutputId(1),
                    name: "DP-1".to_owned(),
                    x: 100,
                    y: 200,
                    width: 2560,
                    height: 1440,
                    scale: 2,
                    refresh_millihz: 60_000,
                }],
            },
            surface_snapshots: PlatformSurfaceSnapshotState {
                surfaces: std::collections::BTreeMap::from([(
                    42,
                    nekoland_ecs::resources::PlatformSurfaceSnapshot {
                        surface_id: 42,
                        kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                        buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                        dmabuf_format: None,
                        import_strategy:
                            nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                        attached: true,
                        scale: 2,
                        content_version: 7,
                    },
                )]),
            },
            ..Default::default()
        });
        main_world.insert_resource(ShellRenderInput {
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: std::collections::BTreeMap::from([(
                    42,
                    nekoland_ecs::resources::SurfacePresentationState {
                        visible: true,
                        target_output: None,
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: 100,
                            height: 100,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: nekoland_ecs::resources::SurfacePresentationRole::Window,
                    },
                )]),
            },
            pending_screenshot_requests:
                nekoland_ecs::resources::PendingScreenshotRequests::default(),
            ..Default::default()
        });
        main_world.spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 42 },
            content_version: SurfaceContentVersion { value: 7 },
            ..Default::default()
        });

        let mut render_world = bevy_ecs::world::World::default();
        extract_render_subapp_inputs(&mut main_world, &mut render_world);

        let views = &render_world.resource::<RenderViewSnapshot>().views;
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].x, 100);
        assert_eq!(views[0].y, 200);
        assert_eq!(views[0].width, 2560);
        assert_eq!(views[0].height, 1440);
        assert_eq!(views[0].scale, 2);

        let versions = &render_world.resource::<SurfaceContentVersionSnapshot>().versions;
        assert_eq!(versions.get(&42), Some(&7));

        let attachments = &render_world.resource::<SurfaceBufferAttachmentSnapshot>().surfaces;
        let attachment = attachments.get(&42).expect("surface attachment snapshot");
        assert!(attachment.attached);
        assert_eq!(attachment.scale, 2);

        let ordered = &render_world
            .resource::<crate::compositor_render::DesktopSurfaceOrderSnapshot>()
            .outputs;
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered.values().next().expect("ordered surfaces"), &vec![42]);
    }

    #[test]
    fn render_subapp_extract_builds_scene_process_snapshots_from_mailboxes() {
        let mut main_world = bevy_ecs::world::World::default();
        main_world.insert_resource(CompositorClock { frame: 1, uptime_millis: 50 });
        main_world.insert_resource(AnimationTimelineStore::default());
        main_world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![OutputGeometrySnapshot {
                    output_id: OutputId(3),
                    name: "Virtual-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                    scale: 1,
                    refresh_millihz: 60_000,
                }],
            },
            ..Default::default()
        });
        main_world.insert_resource(ShellRenderInput {
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: std::collections::BTreeMap::from([(
                    13,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(OutputId(3)),
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: 50,
                            height: 50,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                )]),
            },
            pending_screenshot_requests:
                nekoland_ecs::resources::PendingScreenshotRequests::default(),
            ..Default::default()
        });
        main_world.spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 13 },
            window: XdgWindow::default(),
            ..Default::default()
        });
        main_world.resource_mut::<AnimationTimelineStore>().upsert_track(
            AnimationBindingKey::Source(RenderSourceKey::window(13)),
            AnimationTrack {
                property: AnimationProperty::Opacity,
                from: AnimationValue::Float(0.0),
                to: AnimationValue::Float(1.0),
                start_uptime_millis: 0,
                duration_millis: 100,
                easing: AnimationEasing::Linear,
            },
        );
        main_world.resource_mut::<AnimationTimelineStore>().upsert_track(
            AnimationBindingKey::Instance(RenderInstanceKey::new(
                RenderSourceKey::window(13),
                OutputId(3),
                0,
            )),
            AnimationTrack {
                property: AnimationProperty::Rect,
                from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 50, height: 50 }),
                to: AnimationValue::Rect(RenderRect { x: 10, y: 20, width: 60, height: 70 }),
                start_uptime_millis: 0,
                duration_millis: 100,
                easing: AnimationEasing::Linear,
            },
        );

        let mut advance =
            bevy_ecs::system::IntoSystem::into_system(advance_animation_timelines_system);
        advance.initialize(&mut main_world);
        let _ = advance.run((), &mut main_world);

        let mut render_world = bevy_ecs::world::World::default();
        extract_render_subapp_inputs(&mut main_world, &mut render_world);

        let appearance = render_world.resource::<AppearanceSnapshot>();
        let projection = render_world.resource::<ProjectionSnapshot>();
        assert_eq!(
            appearance.sources.get(&RenderSourceKey::window(13)).map(|state| state.opacity),
            Some(0.5)
        );
        assert_eq!(
            projection
                .instances
                .get(&RenderInstanceKey::new(RenderSourceKey::window(13), OutputId(3), 0))
                .and_then(|state| state.rect_override),
            Some(RenderRect { x: 5, y: 10, width: 55, height: 60 })
        );
    }

    #[test]
    fn compiled_output_frames_mirror_render_outputs() {
        let mut world = bevy_ecs::world::World::default();
        world.insert_resource(OutputDamageRegions::default());
        world.insert_resource(PreparedSceneResources::default());
        world.insert_resource(RenderMaterialFrameState::default());
        world.insert_resource(RenderPassGraph::default());
        world.insert_resource(RenderPlan::default());
        world.insert_resource(RenderProcessPlan::default());
        world.insert_resource(RenderFinalOutputPlan::default());
        world.insert_resource(RenderReadbackPlan::default());
        world.insert_resource(RenderTargetAllocationPlan::default());
        world.insert_resource(SurfaceTextureBridgePlan::default());
        world.insert_resource(PreparedGpuResources::default());
        world.init_resource::<CompiledOutputFrames>();

        let mut system = IntoSystem::into_system(sync_compiled_output_frames_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let compiled = world.resource::<CompiledOutputFrames>();
        assert!(compiled.outputs.is_empty());
        assert_eq!(compiled.output_damage_regions, OutputDamageRegions::default());
        assert_eq!(compiled.prepared_scene, PreparedSceneResources::default());
        assert_eq!(compiled.materials, RenderMaterialFrameState::default());
        assert_eq!(compiled.render_graph, RenderPassGraph::default());
        assert_eq!(compiled.render_plan, RenderPlan::default());
        assert_eq!(compiled.process_plan, RenderProcessPlan::default());
        assert_eq!(compiled.final_output_plan, RenderFinalOutputPlan::default());
        assert_eq!(compiled.render_target_allocation, RenderTargetAllocationPlan::default());
        assert_eq!(compiled.surface_texture_bridge, SurfaceTextureBridgePlan::default());
    }

    #[test]
    fn compiled_output_frames_include_per_output_frames() {
        let mut world = bevy_ecs::world::World::default();
        world.insert_resource(OutputDamageRegions {
            regions: std::collections::BTreeMap::from([(
                nekoland_ecs::components::OutputId(1),
                vec![nekoland_ecs::resources::DamageRect { x: 0, y: 0, width: 10, height: 10 }],
            )]),
        });
        world.insert_resource(PreparedSceneResources::default());
        world.insert_resource(RenderMaterialFrameState::default());
        world.insert_resource(RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                nekoland_ecs::components::OutputId(1),
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(nekoland_ecs::components::OutputId(1)),
                    )]),
                    passes: std::collections::BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::scene(
                            RenderSceneRole::Desktop,
                            RenderTargetId(1),
                            Vec::new(),
                            vec![RenderItemId(1)],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                },
            )]),
        });
        world.insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                nekoland_ecs::components::OutputId(1),
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: RenderItemIdentity::new(RenderSourceId(1), RenderItemId(1)),
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
        world.insert_resource(RenderProcessPlan::default());
        world.insert_resource(RenderFinalOutputPlan::default());
        world.insert_resource(RenderReadbackPlan::default());
        world.insert_resource(RenderTargetAllocationPlan::default());
        world.insert_resource(SurfaceTextureBridgePlan::default());
        world.insert_resource(PreparedGpuResources::default());
        world.init_resource::<CompiledOutputFrames>();

        let mut system = IntoSystem::into_system(sync_compiled_output_frames_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let compiled = world.resource::<CompiledOutputFrames>();
        let output = compiled
            .outputs
            .get(&nekoland_ecs::components::OutputId(1))
            .expect("compiled output frame");
        assert_eq!(output.render_plan.ordered_item_ids(), &[RenderItemId(1)]);
        assert!(output.prepared_scene.items.is_empty());
        assert_eq!(output.execution_plan.ordered_passes, vec![RenderPassId(1)]);
        assert_eq!(output.damage_regions.len(), 1);
    }

    #[test]
    fn stable_ids_flow_from_platform_mailboxes_into_compiled_output_frames() {
        let output_id = OutputId(3);
        let surface_id = 13_u64;

        let mut main_world = bevy_ecs::world::World::default();
        main_world.insert_resource(nekoland_ecs::resources::WindowStackingState {
            workspaces: std::collections::BTreeMap::from([(
                nekoland_ecs::resources::UNASSIGNED_WORKSPACE_STACK_ID,
                vec![surface_id],
            )]),
        });
        main_world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![OutputGeometrySnapshot {
                    output_id,
                    name: "Virtual-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                    scale: 1,
                    refresh_millihz: 60_000,
                }],
            },
            surface_snapshots: PlatformSurfaceSnapshotState {
                surfaces: std::collections::BTreeMap::from([(
                    surface_id,
                    nekoland_ecs::resources::PlatformSurfaceSnapshot {
                        surface_id,
                        kind: nekoland_ecs::resources::PlatformSurfaceKind::Toplevel,
                        buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                        dmabuf_format: None,
                        import_strategy:
                            nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                        attached: true,
                        scale: 1,
                        content_version: 4,
                    },
                )]),
            },
            ..Default::default()
        });
        main_world.insert_resource(ShellRenderInput {
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: std::collections::BTreeMap::from([(
                    surface_id,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(output_id),
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 10,
                            y: 20,
                            width: 100,
                            height: 80,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                )]),
            },
            ..Default::default()
        });
        main_world.spawn(WindowBundle {
            surface: WlSurfaceHandle { id: surface_id },
            content_version: SurfaceContentVersion { value: 4 },
            ..Default::default()
        });

        let mut render_world = bevy_ecs::world::World::default();
        extract_render_subapp_inputs(&mut main_world, &mut render_world);

        assert_eq!(
            render_world.resource::<RenderViewSnapshot>().view(output_id).map(|view| view.output_id),
            Some(output_id)
        );
        assert_eq!(
            render_world
                .resource::<crate::compositor_render::DesktopSurfaceOrderSnapshot>()
                .outputs
                .get(&output_id),
            Some(&vec![surface_id])
        );

        render_world.insert_resource(OutputDamageRegions::default());
        render_world.insert_resource(PreparedSceneResources::default());
        render_world.insert_resource(RenderMaterialFrameState::default());
        render_world.insert_resource(RenderPassGraph {
            outputs: std::collections::BTreeMap::from([(
                output_id,
                OutputExecutionPlan {
                    targets: std::collections::BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(output_id),
                    )]),
                    passes: std::collections::BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::scene(
                            RenderSceneRole::Desktop,
                            RenderTargetId(1),
                            Vec::new(),
                            vec![RenderItemId(1)],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                },
            )]),
        });
        render_world.insert_resource(RenderPlan {
            outputs: std::collections::BTreeMap::from([(
                output_id,
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: RenderItemIdentity::new(RenderSourceId(surface_id), RenderItemId(1)),
                    surface_id,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 10, y: 20, width: 100, height: 80 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                })]),
            )]),
        });
        render_world.insert_resource(RenderProcessPlan::default());
        render_world.insert_resource(RenderFinalOutputPlan::default());
        render_world.insert_resource(RenderReadbackPlan::default());
        render_world.insert_resource(RenderTargetAllocationPlan::default());
        render_world.insert_resource(SurfaceTextureBridgePlan::default());
        render_world.insert_resource(PreparedGpuResources::default());
        render_world.init_resource::<CompiledOutputFrames>();

        let mut sync = bevy_ecs::system::IntoSystem::into_system(sync_compiled_output_frames_system);
        sync.initialize(&mut render_world);
        let _ = sync.run((), &mut render_world);

        let compiled = render_world.resource::<CompiledOutputFrames>();
        let compiled_output = compiled.output(output_id).expect("compiled output should exist");
        let compiled_surface = compiled_output
            .render_plan
            .iter_ordered()
            .find_map(|item| match item {
                RenderPlanItem::Surface(item) => Some(item.surface_id),
                _ => None,
            });
        assert_eq!(compiled_surface, Some(surface_id));
    }
}
