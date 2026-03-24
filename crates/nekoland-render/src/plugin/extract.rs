//! Extraction bridge from the main shell world into the render sub-app.
//!
//! The render sub-app only sees immutable snapshots. This module is responsible for cloning the
//! shell-owned inputs and deriving render-world snapshots that avoid cross-world `Entity` access.

use bevy_ecs::world::World;
use nekoland_ecs::resources::{
    CompositorClock, CompositorSceneState, OutputDamageRegions, ShellRenderInput,
    SurfaceBufferAttachmentSnapshot, SurfaceBufferAttachmentState, SurfaceContentVersionSnapshot,
    WaylandIngress,
};

use crate::{compositor_render, effects, material, scene_process};

pub(super) fn extract_render_subapp_inputs(main_world: &mut World, render_world: &mut World) {
    super::clone_resource_into::<material::RenderMaterialRegistry>(main_world, render_world);
    super::clone_resource_into::<material::RenderMaterialParamsStore>(main_world, render_world);
    super::clone_resource_into::<material::RenderMaterialRequestQueue>(main_world, render_world);
    super::clone_resource_into::<ShellRenderInput>(main_world, render_world);
    super::clone_resource_into::<OutputDamageRegions>(main_world, render_world);
    super::clone_resource_into::<CompositorSceneState>(main_world, render_world);
    super::clone_resource_into::<CompositorClock>(main_world, render_world);
    super::clone_resource_into::<effects::blur::BlurEffectConfig>(main_world, render_world);
    super::clone_resource_into::<effects::shadow::ShadowEffectConfig>(main_world, render_world);
    super::clone_resource_into::<effects::rounded_corners::RoundedCornerEffectConfig>(
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

fn sync_render_platform_inputs_from_wayland_ingress(main_world: &World, render_world: &mut World) {
    let wayland_ingress = main_world.resource::<WaylandIngress>();
    render_world.insert_resource(wayland_ingress.surface_snapshots.clone());
}

fn extract_render_view_snapshot(main_world: &mut World, render_world: &mut World) {
    let views = main_world
        .resource::<WaylandIngress>()
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
        .collect::<Vec<_>>();
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
        bevy_ecs::query::With<nekoland_ecs::components::PopupSurface>,
    >();
    let mut workspaces =
        main_world.query::<(bevy_ecs::entity::Entity, nekoland_ecs::views::WorkspaceRuntime)>();

    let stacking = main_world.resource::<nekoland_ecs::resources::WindowStackingState>().clone();
    let shell_render_input = main_world.resource::<ShellRenderInput>().clone();
    let surface_presentation = &shell_render_input.surface_presentation;
    let live_outputs = main_world
        .resource::<WaylandIngress>()
        .output_snapshots
        .outputs
        .iter()
        .map(|output| output.output_id)
        .collect::<Vec<_>>();
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
            let state = surface_presentation.surfaces.get(&window.surface_id());
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
            let state = surface_presentation.surfaces.get(&window.surface_id());
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
                .surfaces
                .get(&layer.surface_id())
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
                .surfaces
                .get(&popup.surface_id())
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
                .surfaces
                .get(&layer.surface_id())
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
        let Some(state) = surface_presentation.surfaces.get(&surface_id) else {
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
        .resource::<WaylandIngress>()
        .surface_snapshots
        .surfaces
        .iter()
        .map(|(surface_id, surface)| (*surface_id, surface.content_version))
        .collect();
    render_world.insert_resource(SurfaceContentVersionSnapshot { versions });
}

fn extract_surface_buffer_attachment_snapshot(main_world: &mut World, render_world: &mut World) {
    let snapshot = main_world
        .resource::<WaylandIngress>()
        .surface_snapshots
        .surfaces
        .iter()
        .map(|(surface_id, surface)| {
            (
                *surface_id,
                SurfaceBufferAttachmentState {
                    attached: surface.attached,
                    scale: surface.scale,
                },
            )
        })
        .collect();
    render_world.insert_resource(SurfaceBufferAttachmentSnapshot { surfaces: snapshot });
}
