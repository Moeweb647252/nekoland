//! Desktop scene extraction and render-plan assembly.
//!
//! This module converts shell-owned visibility and ordering state into output-local render scene
//! contributions, then assembles those contributions into stable render-plan items.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{PopupSurface, XdgWindow};
use nekoland_ecs::presentation_logic::{
    is_background_band_layer, is_foreground_band_layer, managed_window_visible, popup_visible,
};
use nekoland_ecs::resources::{
    OutputRenderPlan, RenderItemInstance, RenderPlan, RenderRect, RenderSceneRole,
    ShellRenderInput, SurfacePresentationRole, UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
};
use nekoland_ecs::views::{
    LayerRenderRuntime, OutputRuntime, PopupRenderRuntime, WindowRenderRuntime, WorkspaceRuntime,
};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::scene_process::{
    AppearanceSnapshot, ProjectionSnapshot, apply_appearance_snapshot, apply_projection_snapshot,
};
use crate::scene_source::{
    RenderInstanceKey, RenderSceneContribution, RenderSceneContributionQueue,
    RenderSceneIdentityRegistry, RenderSourceKey, contribution_to_plan_item,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameComposer;

#[derive(bevy_ecs::prelude::Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderViewSnapshot {
    pub views: Vec<RenderViewState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderViewState {
    pub output_id: nekoland_ecs::components::OutputId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
}

impl RenderViewSnapshot {
    pub fn ids(&self) -> impl Iterator<Item = nekoland_ecs::components::OutputId> + '_ {
        self.views.iter().map(|view| view.output_id)
    }

    pub fn view(&self, output_id: nekoland_ecs::components::OutputId) -> Option<&RenderViewState> {
        self.views.iter().find(|view| view.output_id == output_id)
    }
}

#[derive(bevy_ecs::prelude::Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopSurfaceOrderSnapshot {
    pub outputs: BTreeMap<nekoland_ecs::components::OutputId, Vec<u64>>,
}

#[derive(SystemParam)]
pub struct FrameCompositionInputs<'w, 's> {
    outputs: Query<'w, 's, OutputRuntime>,
    layers: Query<'w, 's, LayerRenderRuntime, With<nekoland_ecs::components::LayerShellSurface>>,
    windows: Query<'w, 's, (Entity, WindowRenderRuntime), With<XdgWindow>>,
    popups: Query<'w, 's, PopupRenderRuntime, With<PopupSurface>>,
    stacking: Res<'w, WindowStackingState>,
    workspaces: Query<'w, 's, (Entity, WorkspaceRuntime)>,
    shell_render_input: Res<'w, ShellRenderInput>,
    appearance: Option<Res<'w, AppearanceSnapshot>>,
    projection: Option<Res<'w, ProjectionSnapshot>>,
    scene_contributions: ResMut<'w, RenderSceneContributionQueue>,
}

#[derive(SystemParam)]
pub struct RenderPlanAssemblyInputs<'w, 's> {
    outputs: Query<'w, 's, OutputRuntime>,
    scene_contributions: Res<'w, RenderSceneContributionQueue>,
    identity_registry: ResMut<'w, RenderSceneIdentityRegistry>,
    render_plan: ResMut<'w, RenderPlan>,
}

#[derive(SystemParam)]
pub struct RenderPlanSnapshotAssemblyInputs<'w> {
    views: Res<'w, RenderViewSnapshot>,
    scene_contributions: Res<'w, RenderSceneContributionQueue>,
    identity_registry: ResMut<'w, RenderSceneIdentityRegistry>,
    render_plan: ResMut<'w, RenderPlan>,
}

#[derive(SystemParam)]
pub struct DesktopSurfaceOrderInputs<'w, 's> {
    outputs: Query<'w, 's, OutputRuntime>,
    layers: Query<'w, 's, LayerRenderRuntime, With<nekoland_ecs::components::LayerShellSurface>>,
    windows: Query<'w, 's, (Entity, WindowRenderRuntime), With<XdgWindow>>,
    popups: Query<'w, 's, PopupRenderRuntime, With<PopupSurface>>,
    stacking: Res<'w, WindowStackingState>,
    workspaces: Query<'w, 's, (Entity, WorkspaceRuntime)>,
    shell_render_input: Res<'w, ShellRenderInput>,
    ordered_surfaces: ResMut<'w, DesktopSurfaceOrderSnapshot>,
}

/// Builds the per-frame output-scoped render plan from already-laid-out surfaces plus projected
/// visual state.
///
/// Composition order is deliberate: output backgrounds, background/bottom layers, visible
/// windows, popups whose parents are still visible, then top/overlay layers.
pub fn emit_desktop_scene_contributions_system(composition: FrameCompositionInputs<'_, '_>) {
    let FrameCompositionInputs {
        outputs,
        layers,
        windows,
        popups,
        stacking,
        workspaces,
        shell_render_input,
        appearance,
        projection,
        mut scene_contributions,
    } = composition;
    let surface_presentation = Some(&shell_render_input.surface_presentation);
    let appearance = appearance.as_deref();
    let projection = projection.as_deref();
    let live_outputs = outputs.iter().map(|output| output.id()).collect::<Vec<_>>();
    let mut contributions = live_outputs
        .iter()
        .copied()
        .map(|output_id| (output_id, Vec::new()))
        .collect::<BTreeMap<_, Vec<RenderSceneContribution>>>();
    let background_windows = windows
        .iter()
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
                |state| state.visible && state.role == SurfacePresentationRole::OutputBackground,
            );
            if !visible {
                return None;
            }
            let output_id = state
                .and_then(|state| state.target_output)
                .or_else(|| window.background.as_ref().map(|background| background.output))?;
            Some((output_id, window.surface_id()))
        })
        .fold(BTreeMap::new(), |mut backgrounds, (output_id, candidate)| {
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
        .iter()
        .filter_map(|(entity, window)| {
            let state = surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&window.surface_id()));
            let visible = state.map_or_else(
                || {
                    managed_window_visible(
                        *window.mode,
                        window.viewport_visibility.visible,
                        *window.role,
                    )
                },
                |state| state.visible && state.role == SurfacePresentationRole::Window,
            );
            visible.then_some((
                entity,
                window.surface_id(),
                window_workspace_runtime_id(window.child_of, &workspaces)
                    .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
            ))
        })
        .collect::<Vec<_>>();
    let active_window_entities =
        visible_windows.iter().map(|(entity, ..)| *entity).collect::<BTreeSet<_>>();
    let ordered_window_surfaces = stacking.ordered_surfaces(
        visible_windows.iter().map(|(_, surface_id, workspace_id)| (*workspace_id, *surface_id)),
    );
    let elements = background_windows
        .into_iter()
        .chain(
            layers
                .iter()
                .filter(|layer| {
                    surface_presentation
                        .and_then(|snapshot| snapshot.surfaces.get(&layer.surface_id()))
                        .map_or_else(
                            || {
                                layer.buffer.attached
                                    && is_background_band_layer(layer.layer_surface.layer)
                            },
                            |state| {
                                state.visible
                                    && state.role == SurfacePresentationRole::Layer
                                    && is_background_band_layer(layer.layer_surface.layer)
                            },
                        )
                })
                .map(|layer| layer.surface_id()),
        )
        .chain(ordered_window_surfaces)
        .chain(
            popups
                .iter()
                .filter(|popup| {
                    surface_presentation
                        .and_then(|snapshot| snapshot.surfaces.get(&popup.surface_id()))
                        .map_or_else(
                            || {
                                popup_visible(
                                    popup.buffer.attached,
                                    popup_parent_visible(popup.child_of, &active_window_entities),
                                )
                            },
                            |state| state.visible && state.role == SurfacePresentationRole::Popup,
                        )
                })
                .map(|popup| popup.surface_id()),
        )
        .chain(
            layers
                .iter()
                .filter(|layer| {
                    surface_presentation
                        .and_then(|snapshot| snapshot.surfaces.get(&layer.surface_id()))
                        .map_or_else(
                            || {
                                layer.buffer.attached
                                    && is_foreground_band_layer(layer.layer_surface.layer)
                            },
                            |state| {
                                state.visible
                                    && state.role == SurfacePresentationRole::Layer
                                    && is_foreground_band_layer(layer.layer_surface.layer)
                            },
                        )
                })
                .map(|layer| layer.surface_id()),
        )
        .collect::<Vec<_>>();

    for (z_index, surface_id) in elements.into_iter().enumerate() {
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
            let source_key = RenderSourceKey::surface_for_role(surface_id, state.role);
            let instance_key = RenderInstanceKey::new(source_key.clone(), output_id, 0);
            let mut instance = RenderItemInstance {
                rect: RenderRect::from(&state.geometry),
                opacity: 1.0,
                clip_rect: None,
                z_index: z_index as i32,
                scene_role: RenderSceneRole::Desktop,
            };
            apply_appearance_snapshot(
                &mut instance.opacity,
                &source_key,
                &instance_key,
                appearance,
            );
            apply_projection_snapshot(
                &mut instance.rect,
                &mut instance.clip_rect,
                &source_key,
                &instance_key,
                projection,
            );
            contributions.entry(output_id).or_default().push(RenderSceneContribution::surface(
                output_id, source_key, surface_id, 0, instance,
            ));
        }
    }

    tracing::trace!(
        outputs = contributions.len(),
        elements = contributions.values().map(Vec::len).sum::<usize>(),
        "desktop scene contribution tick"
    );
    scene_contributions.outputs = contributions;
}

/// Resolves stable render item ids and assembles the final output-scoped render plan.
pub fn assemble_render_plan_system(assembly: RenderPlanAssemblyInputs<'_, '_>) {
    let RenderPlanAssemblyInputs {
        outputs,
        scene_contributions,
        mut identity_registry,
        mut render_plan,
    } = assembly;

    let mut plans = outputs
        .iter()
        .map(|output| (output.id(), OutputRenderPlan::default()))
        .collect::<BTreeMap<_, _>>();

    for (output_id, output_contributions) in &scene_contributions.outputs {
        let output_plan = plans.entry(*output_id).or_default();
        for contribution in output_contributions {
            output_plan.insert(contribution_to_plan_item(contribution, &mut identity_registry));
        }
        output_plan.sort_by_z_index();
    }

    render_plan.outputs = plans;

    tracing::trace!(
        outputs = render_plan.outputs.len(),
        elements = render_plan.outputs.values().map(|plan| plan.items.len()).sum::<usize>(),
        "render plan assembly tick"
    );
}

pub fn assemble_render_plan_from_snapshot_system(assembly: RenderPlanSnapshotAssemblyInputs<'_>) {
    let RenderPlanSnapshotAssemblyInputs {
        views,
        scene_contributions,
        mut identity_registry,
        mut render_plan,
    } = assembly;

    let mut plans = views
        .ids()
        .map(|output_id| (output_id, OutputRenderPlan::default()))
        .collect::<BTreeMap<_, _>>();

    for (output_id, output_contributions) in &scene_contributions.outputs {
        let output_plan = plans.entry(*output_id).or_default();
        for contribution in output_contributions {
            output_plan.insert(contribution_to_plan_item(contribution, &mut identity_registry));
        }
        output_plan.sort_by_z_index();
    }

    render_plan.outputs = plans;
}

pub fn snapshot_desktop_surface_order_system(inputs: DesktopSurfaceOrderInputs<'_, '_>) {
    let DesktopSurfaceOrderInputs {
        outputs,
        layers,
        windows,
        popups,
        stacking,
        workspaces,
        shell_render_input,
        mut ordered_surfaces,
    } = inputs;
    let surface_presentation = Some(&shell_render_input.surface_presentation);
    let live_outputs = outputs.iter().map(|output| output.id()).collect::<Vec<_>>();
    let mut ordered = live_outputs
        .iter()
        .copied()
        .map(|output_id| (output_id, Vec::new()))
        .collect::<BTreeMap<_, Vec<u64>>>();
    let background_windows = windows
        .iter()
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
                |state| state.visible && state.role == SurfacePresentationRole::OutputBackground,
            );
            if !visible {
                return None;
            }
            let output_id = state
                .and_then(|state| state.target_output)
                .or_else(|| window.background.as_ref().map(|background| background.output))?;
            Some((output_id, window.surface_id()))
        })
        .fold(BTreeMap::new(), |mut backgrounds, (output_id, candidate)| {
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
        .iter()
        .filter_map(|(entity, window)| {
            let state = surface_presentation
                .and_then(|snapshot| snapshot.surfaces.get(&window.surface_id()));
            let visible = state.map_or_else(
                || {
                    managed_window_visible(
                        *window.mode,
                        window.viewport_visibility.visible,
                        *window.role,
                    )
                },
                |state| state.visible && state.role == SurfacePresentationRole::Window,
            );
            visible.then_some((
                entity,
                window.surface_id(),
                window_workspace_runtime_id(window.child_of, &workspaces)
                    .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
            ))
        })
        .collect::<Vec<_>>();
    let active_window_entities =
        visible_windows.iter().map(|(entity, ..)| *entity).collect::<BTreeSet<_>>();
    let ordered_window_surfaces = stacking.ordered_surfaces(
        visible_windows.iter().map(|(_, surface_id, workspace_id)| (*workspace_id, *surface_id)),
    );
    let elements = background_windows
        .into_iter()
        .chain(
            layers
                .iter()
                .filter(|layer| {
                    surface_presentation
                        .and_then(|snapshot| snapshot.surfaces.get(&layer.surface_id()))
                        .map_or_else(
                            || {
                                layer.buffer.attached
                                    && is_background_band_layer(layer.layer_surface.layer)
                            },
                            |state| {
                                state.visible
                                    && state.role == SurfacePresentationRole::Layer
                                    && is_background_band_layer(layer.layer_surface.layer)
                            },
                        )
                })
                .map(|layer| layer.surface_id()),
        )
        .chain(ordered_window_surfaces)
        .chain(
            popups
                .iter()
                .filter(|popup| {
                    surface_presentation
                        .and_then(|snapshot| snapshot.surfaces.get(&popup.surface_id()))
                        .map_or_else(
                            || {
                                popup_visible(
                                    popup.buffer.attached,
                                    popup_parent_visible(popup.child_of, &active_window_entities),
                                )
                            },
                            |state| state.visible && state.role == SurfacePresentationRole::Popup,
                        )
                })
                .map(|popup| popup.surface_id()),
        )
        .chain(
            layers
                .iter()
                .filter(|layer| {
                    surface_presentation
                        .and_then(|snapshot| snapshot.surfaces.get(&layer.surface_id()))
                        .map_or_else(
                            || {
                                layer.buffer.attached
                                    && is_foreground_band_layer(layer.layer_surface.layer)
                            },
                            |state| {
                                state.visible
                                    && state.role == SurfacePresentationRole::Layer
                                    && is_foreground_band_layer(layer.layer_surface.layer)
                            },
                        )
                })
                .map(|layer| layer.surface_id()),
        )
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

    ordered_surfaces.outputs = ordered;
}

pub fn emit_desktop_scene_contributions_from_snapshot_system(
    views: Res<'_, RenderViewSnapshot>,
    ordered_surfaces: Res<'_, DesktopSurfaceOrderSnapshot>,
    shell_render_input: Res<'_, ShellRenderInput>,
    appearance: Option<Res<'_, AppearanceSnapshot>>,
    projection: Option<Res<'_, ProjectionSnapshot>>,
    mut scene_contributions: ResMut<'_, RenderSceneContributionQueue>,
) {
    let surface_presentation = Some(&shell_render_input.surface_presentation);
    let appearance = appearance.as_deref();
    let projection = projection.as_deref();
    let mut contributions = views
        .ids()
        .map(|output_id| (output_id, Vec::new()))
        .collect::<BTreeMap<_, Vec<RenderSceneContribution>>>();

    for (output_id, surfaces) in &ordered_surfaces.outputs {
        for (z_index, surface_id) in surfaces.iter().copied().enumerate() {
            let Some(state) =
                surface_presentation.and_then(|snapshot| snapshot.surfaces.get(&surface_id))
            else {
                continue;
            };
            if !state.visible {
                continue;
            }
            let source_key = RenderSourceKey::surface_for_role(surface_id, state.role);
            let instance_key = RenderInstanceKey::new(source_key.clone(), *output_id, 0);
            let mut instance = RenderItemInstance {
                rect: RenderRect::from(&state.geometry),
                opacity: 1.0,
                clip_rect: None,
                z_index: z_index as i32,
                scene_role: RenderSceneRole::Desktop,
            };
            apply_appearance_snapshot(
                &mut instance.opacity,
                &source_key,
                &instance_key,
                appearance,
            );
            apply_projection_snapshot(
                &mut instance.rect,
                &mut instance.clip_rect,
                &source_key,
                &instance_key,
                projection,
            );
            contributions.entry(*output_id).or_default().push(RenderSceneContribution::surface(
                *output_id, source_key, surface_id, 0, instance,
            ));
        }
    }

    scene_contributions.outputs = contributions;
}

fn popup_parent_visible(child_of: &ChildOf, active_window_entities: &BTreeSet<Entity>) -> bool {
    active_window_entities.contains(&child_of.parent())
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::{PreRenderSchedule, RenderSchedule};
    use nekoland_ecs::bundles::{LayerSurfaceBundle, OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        LayerLevel, OutputBackgroundWindow, OutputDevice, OutputKind, OutputProperties,
        WindowAnimation, WindowRole, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{
        CompositorSceneEntry, CompositorSceneEntryId, CompositorSceneState, OutputCompositorScene,
        OutputOverlaySpec, OutputOverlayState, RenderColor, RenderItemIdentity, RenderItemInstance,
        RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole, RenderSourceId, ShellRenderInput,
        SurfacePresentationRole, SurfacePresentationSnapshot, SurfacePresentationState,
        UNASSIGNED_WORKSPACE_STACK_ID, WaylandIngress, WindowStackingState,
    };

    use crate::animation::{
        AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
        AnimationTrack, AnimationValue,
    };
    use crate::scene_process::{AppearanceSnapshot, AppearanceState, ProjectionSnapshot};
    use crate::scene_source::{RenderInstanceKey, RenderSceneContributionQueue, RenderSourceKey};

    use super::{assemble_render_plan_system, emit_desktop_scene_contributions_system};

    fn add_render_plan_systems(app: &mut NekolandApp) {
        app.inner_mut()
            .init_resource::<crate::animation::AnimationTimelineStore>()
            .init_resource::<AppearanceSnapshot>()
            .init_resource::<ProjectionSnapshot>()
            .init_resource::<CompositorSceneState>()
            .init_resource::<ShellRenderInput>()
            .init_resource::<WaylandIngress>()
            .init_resource::<OutputOverlayState>()
            .init_resource::<crate::output_overlay::OutputOverlaySceneSyncState>()
            .init_resource::<RenderSceneContributionQueue>()
            .init_resource::<crate::scene_source::RenderSceneIdentityRegistry>()
            .add_systems(
                PreRenderSchedule,
                (
                    crate::scene_process::clear_scene_process_snapshots_system,
                    crate::scene_process::surface_scene_process_snapshot_system,
                    crate::scene_process::compositor_scene_process_snapshot_system,
                )
                    .chain(),
            )
            .add_systems(
                RenderSchedule,
                (
                    crate::scene_source::clear_scene_contributions_system,
                    emit_desktop_scene_contributions_system,
                    crate::output_overlay::sync_output_overlay_scene_state_system,
                    crate::scene_source::emit_compositor_scene_contributions_system,
                    assemble_render_plan_system,
                )
                    .chain(),
            );
    }

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), nekoland_ecs::resources::RenderItemId(id))
    }

    fn spawn_default_output(app: &mut NekolandApp) -> nekoland_ecs::components::OutputId {
        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Nekoland".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        app.inner()
            .world()
            .get::<nekoland_ecs::components::OutputId>(output)
            .copied()
            .map(|output_id| {
                app.inner_mut()
                    .world_mut()
                    .resource_mut::<WaylandIngress>()
                    .output_snapshots
                    .outputs = vec![nekoland_ecs::resources::OutputGeometrySnapshot {
                    output_id,
                    name: "Virtual-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                    scale: 1,
                    refresh_millihz: 60_000,
                }];
                output_id
            })
            .expect("output id should exist")
    }

    fn single_output_surface_order(app: &NekolandApp) -> Vec<u64> {
        app.inner()
            .world()
            .resource::<RenderPlan>()
            .outputs
            .values()
            .next()
            .into_iter()
            .flat_map(|plan| plan.iter_ordered())
            .filter_map(|item| match item {
                RenderPlanItem::Surface(item) => Some(item.surface_id),
                RenderPlanItem::Quad(_)
                | RenderPlanItem::Backdrop(_)
                | RenderPlanItem::Cursor(_) => None,
            })
            .collect()
    }

    fn set_surface_states(
        app: &mut NekolandApp,
        states: impl IntoIterator<Item = (u64, SurfacePresentationRole)>,
    ) {
        let world = app.inner_mut().world_mut();
        let target_output = {
            let mut named_outputs =
                world.query::<(&nekoland_ecs::components::OutputId, &nekoland_ecs::components::OutputDevice)>();
            named_outputs
                .iter(world)
                .find(|(_, output)| output.name == "Virtual-1")
                .map(|(output_id, _)| *output_id)
                .or_else(|| {
                    let mut output_ids = world.query::<&nekoland_ecs::components::OutputId>();
                    output_ids.iter(world).copied().next()
                })
                .expect("render tests should seed at least one output")
        };
        let snapshot = states
            .into_iter()
            .map(|(surface_id, role)| {
                (
                    surface_id,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(target_output),
                        geometry: nekoland_ecs::components::SurfaceGeometry::default(),
                        input_enabled: true,
                        damage_enabled: true,
                        role,
                    },
                )
            })
            .collect();
        world.insert_resource(SurfacePresentationSnapshot { surfaces: snapshot });
        world.resource_mut::<ShellRenderInput>().surface_presentation =
            world.resource::<SurfacePresentationSnapshot>().clone();
    }

    #[test]
    fn render_order_follows_window_stacking_state() {
        let mut app = NekolandApp::new("render-stack-order-test");
        app.inner_mut().init_resource::<RenderPlan>().insert_resource(WindowStackingState {
            workspaces: std::collections::BTreeMap::from([(
                UNASSIGNED_WORKSPACE_STACK_ID,
                vec![22, 11],
            )]),
        });
        add_render_plan_systems(&mut app);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            window: XdgWindow { app_id: "org.nekoland.test".to_owned(), title: "front".to_owned() },
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            window: XdgWindow { app_id: "org.nekoland.test".to_owned(), title: "back".to_owned() },
            ..Default::default()
        });
        spawn_default_output(&mut app);
        set_surface_states(
            &mut app,
            [(11, SurfacePresentationRole::Window), (22, SurfacePresentationRole::Window)],
        );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = single_output_surface_order(&app);
        assert_eq!(render_order, vec![22, 11]);
    }

    #[test]
    fn background_windows_render_below_normal_windows() {
        let mut app = NekolandApp::new("render-background-order-test");
        app.inner_mut().init_resource::<RenderPlan>().insert_resource(WindowStackingState {
            workspaces: std::collections::BTreeMap::from([(
                UNASSIGNED_WORKSPACE_STACK_ID,
                vec![22],
            )]),
        });
        add_render_plan_systems(&mut app);
        let output_id = spawn_default_output(&mut app);

        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                window: XdgWindow {
                    app_id: "org.nekoland.test".to_owned(),
                    title: "background".to_owned(),
                },
                ..Default::default()
            },
            WindowRole::OutputBackground,
            OutputBackgroundWindow {
                output: output_id,
                restore: nekoland_ecs::components::WindowRestoreState {
                    geometry: Default::default(),
                    layout: nekoland_ecs::components::WindowLayout::Floating,
                    mode: nekoland_ecs::components::WindowMode::Normal,
                    fullscreen_output: None,
                    previous: None,
                },
            },
        ));
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            window: XdgWindow { app_id: "org.nekoland.test".to_owned(), title: "front".to_owned() },
            ..Default::default()
        });
        set_surface_states(
            &mut app,
            [
                (11, SurfacePresentationRole::OutputBackground),
                (22, SurfacePresentationRole::Window),
            ],
        );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = single_output_surface_order(&app);
        assert_eq!(render_order, vec![11, 22]);
    }

    #[test]
    fn background_windows_render_below_background_layers() {
        let mut app = NekolandApp::new("render-background-layer-order-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState::default());
        add_render_plan_systems(&mut app);
        let output_id = spawn_default_output(&mut app);

        app.inner_mut().world_mut().spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                window: XdgWindow {
                    app_id: "org.nekoland.test".to_owned(),
                    title: "background".to_owned(),
                },
                ..Default::default()
            },
            WindowRole::OutputBackground,
            OutputBackgroundWindow {
                output: output_id,
                restore: nekoland_ecs::components::WindowRestoreState {
                    geometry: Default::default(),
                    layout: nekoland_ecs::components::WindowLayout::Floating,
                    mode: nekoland_ecs::components::WindowMode::Normal,
                    fullscreen_output: None,
                    previous: None,
                },
            },
        ));
        app.inner_mut().world_mut().spawn(LayerSurfaceBundle {
            surface: WlSurfaceHandle { id: 22 },
            buffer: nekoland_ecs::components::BufferState { attached: true, scale: 1 },
            layer_surface: nekoland_ecs::components::LayerShellSurface {
                layer: LayerLevel::Background,
                ..Default::default()
            },
            ..Default::default()
        });
        set_surface_states(
            &mut app,
            [(11, SurfacePresentationRole::OutputBackground), (22, SurfacePresentationRole::Layer)],
        );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = single_output_surface_order(&app);
        assert_eq!(render_order, vec![11, 22]);
    }

    #[test]
    fn render_uses_appearance_snapshot_instead_of_animation_component() {
        let mut app = NekolandApp::new("render-appearance-boundary-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .init_resource::<AppearanceSnapshot>()
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11],
                )]),
            });
        add_render_plan_systems(&mut app);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "window".to_owned(),
            },
            animation: WindowAnimation { progress: 0.9, ..Default::default() },
            ..Default::default()
        });
        app.inner_mut()
            .world_mut()
            .resource_mut::<AppearanceSnapshot>()
            .sources
            .insert(RenderSourceKey::window(11), AppearanceState { opacity: 0.25 });
        spawn_default_output(&mut app);
        set_surface_states(&mut app, [(11, SurfacePresentationRole::Window)]);

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_plan = app.inner().world().resource::<RenderPlan>();
        let output_plan = render_plan.outputs.values().next().expect("single output render plan");
        assert_eq!(output_plan.items.len(), 1);
        let RenderPlanItem::Surface(item) =
            output_plan.iter_ordered().next().expect("expected one surface item")
        else {
            panic!("expected surface render item");
        };
        assert_eq!(item.instance.opacity, 0.25);
    }

    #[test]
    fn instance_bound_rect_animation_overrides_scene_rect_without_mutating_presentation_geometry() {
        let mut app = NekolandApp::new("render-instance-bound-rect-override-test");
        app.inner_mut().init_resource::<RenderPlan>().insert_resource(WindowStackingState {
            workspaces: std::collections::BTreeMap::from([(
                UNASSIGNED_WORKSPACE_STACK_ID,
                vec![11],
            )]),
        });
        add_render_plan_systems(&mut app);
        let output_id = spawn_default_output(&mut app);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "window".to_owned(),
            },
            ..Default::default()
        });
        set_surface_states(&mut app, [(11, SurfacePresentationRole::Window)]);
        app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>().upsert_track(
            AnimationBindingKey::Instance(RenderInstanceKey::new(
                RenderSourceKey::window(11),
                output_id,
                0,
            )),
            AnimationTrack {
                property: AnimationProperty::Rect,
                from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 100, height: 80 }),
                to: AnimationValue::Rect(RenderRect { x: 30, y: 40, width: 120, height: 90 }),
                start_uptime_millis: 0,
                duration_millis: 1,
                easing: AnimationEasing::Linear,
            },
        );
        {
            let timelines = app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>();
            let binding = AnimationBindingKey::Instance(RenderInstanceKey::new(
                RenderSourceKey::window(11),
                output_id,
                0,
            ));
            assert_eq!(
                timelines.sampled_value(&binding, AnimationProperty::Rect),
                None,
                "instance-bound override should be consumed from sampled values, not written directly"
            );
        }
        app.inner_mut().world_mut().insert_resource(nekoland_ecs::resources::CompositorClock {
            frame: 1,
            uptime_millis: 1,
        });
        let mut system =
            IntoSystem::into_system(crate::animation::advance_animation_timelines_system);
        system.initialize(app.inner_mut().world_mut());
        let _ = system.run((), app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_plan = app.inner().world().resource::<RenderPlan>();
        let output_plan = render_plan.outputs.get(&output_id).expect("single output render plan");
        let RenderPlanItem::Surface(item) =
            output_plan.iter_ordered().next().expect("expected one surface item")
        else {
            panic!("expected surface render item");
        };
        assert_eq!(item.instance.rect, RenderRect { x: 30, y: 40, width: 120, height: 90 });
        let presentation = app.inner().world().resource::<SurfacePresentationSnapshot>();
        assert_eq!(
            presentation.surfaces.get(&11).map(|state| &state.geometry),
            Some(&nekoland_ecs::components::SurfaceGeometry::default())
        );
    }

    #[test]
    fn only_latest_background_window_per_output_renders() {
        let mut app = NekolandApp::new("render-single-background-per-output-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState::default());
        add_render_plan_systems(&mut app);
        let output_id = spawn_default_output(&mut app);

        for surface_id in [11, 22] {
            app.inner_mut().world_mut().spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: surface_id },
                    window: XdgWindow {
                        app_id: "org.nekoland.test".to_owned(),
                        title: format!("background-{surface_id}"),
                    },
                    ..Default::default()
                },
                WindowRole::OutputBackground,
                OutputBackgroundWindow {
                    output: output_id,
                    restore: nekoland_ecs::components::WindowRestoreState {
                        geometry: Default::default(),
                        layout: nekoland_ecs::components::WindowLayout::Floating,
                        mode: nekoland_ecs::components::WindowMode::Normal,
                        fullscreen_output: None,
                        previous: None,
                    },
                },
            ));
        }
        set_surface_states(
            &mut app,
            [
                (11, SurfacePresentationRole::OutputBackground),
                (22, SurfacePresentationRole::OutputBackground),
            ],
        );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = single_output_surface_order(&app);
        assert_eq!(render_order, vec![22]);
    }

    #[test]
    fn scene_contributions_append_non_surface_items() {
        let mut app = NekolandApp::new("render-plan-injection-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState::default());
        add_render_plan_systems(&mut app);
        let output_id = spawn_default_output(&mut app);
        app.inner_mut().world_mut().resource_mut::<CompositorSceneState>().outputs =
            std::collections::BTreeMap::from([(
                output_id,
                OutputCompositorScene::from_entries([(
                    CompositorSceneEntryId(1),
                    CompositorSceneEntry::solid_color(
                        RenderColor { r: 10, g: 20, b: 30, a: 200 },
                        RenderItemInstance {
                            rect: RenderRect { x: 5, y: 6, width: 40, height: 50 },
                            opacity: 0.75,
                            clip_rect: None,
                            z_index: 3,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    ),
                )]),
            )]);

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let output_plan = app
            .inner()
            .world()
            .resource::<RenderPlan>()
            .outputs
            .get(&output_id)
            .unwrap_or_else(|| panic!("output plan should exist for injected scene items"));
        assert!(matches!(output_plan.iter_ordered().next(), Some(RenderPlanItem::Quad(_))));
        let RenderPlanItem::Quad(item) =
            output_plan.iter_ordered().next().expect("expected one quad item")
        else {
            panic!("expected quad");
        };
        assert_eq!(item.identity, identity(1));
    }

    #[test]
    fn output_overlay_state_syncs_into_render_plan() {
        let mut app = NekolandApp::new("render-plan-output-overlay-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState::default());
        add_render_plan_systems(&mut app);
        let output_id = spawn_default_output(&mut app);
        app.inner_mut().world_mut().resource_mut::<ShellRenderInput>().output_overlays.upsert(
            output_id,
            OutputOverlaySpec::solid_color(
                "debug",
                RenderRect { x: 9, y: 8, width: 70, height: 60 },
                Some(RenderRect { x: 10, y: 11, width: 20, height: 21 }),
                RenderColor { r: 1, g: 2, b: 3, a: 200 },
                0.5,
                7,
            ),
        );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let output_plan = app
            .inner()
            .world()
            .resource::<RenderPlan>()
            .outputs
            .get(&output_id)
            .unwrap_or_else(|| panic!("output plan should exist for overlay state"));
        let RenderPlanItem::Quad(item) =
            output_plan.iter_ordered().next().expect("expected one overlay rect item")
        else {
            panic!("expected quad");
        };
        assert_eq!(item.instance.rect.x, 9);
        assert_eq!(item.instance.clip_rect.map(|rect| rect.width), Some(20));
        assert_eq!(item.instance.opacity, 0.5);
    }
}
