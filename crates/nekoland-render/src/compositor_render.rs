use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{XdgPopup, XdgWindow};
use nekoland_ecs::presentation_logic::{
    is_background_band_layer, is_foreground_band_layer, managed_window_visible, popup_visible,
};
use nekoland_ecs::resources::{
    OutputRenderPlan, RenderItemInstance, RenderPlan, RenderRect, RenderSceneRole,
    SurfacePresentationRole, SurfacePresentationSnapshot, SurfaceVisualSnapshot,
    UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
};
use nekoland_ecs::views::{
    LayerRenderRuntime, OutputRuntime, PopupRenderRuntime, WindowRenderRuntime, WorkspaceRuntime,
};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::scene_source::{
    RenderSceneContribution, RenderSceneContributionQueue, RenderSceneIdentityRegistry,
    contribution_to_plan_item,
};
use crate::{
    animation::{AnimationBindingKey, AnimationProperty, AnimationTimelineStore, AnimationValue},
    scene_source::{RenderInstanceKey, RenderSourceKey},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameComposer;

#[derive(SystemParam)]
pub struct FrameCompositionInputs<'w, 's> {
    outputs: Query<'w, 's, OutputRuntime>,
    layers: Query<'w, 's, LayerRenderRuntime, With<nekoland_ecs::components::LayerShellSurface>>,
    windows: Query<'w, 's, (Entity, WindowRenderRuntime), With<XdgWindow>>,
    popups: Query<'w, 's, PopupRenderRuntime, With<XdgPopup>>,
    stacking: Res<'w, WindowStackingState>,
    workspaces: Query<'w, 's, (Entity, WorkspaceRuntime)>,
    surface_presentation: Option<Res<'w, SurfacePresentationSnapshot>>,
    surface_visual: Option<Res<'w, SurfaceVisualSnapshot>>,
    animation_timelines: Res<'w, AnimationTimelineStore>,
    scene_contributions: ResMut<'w, RenderSceneContributionQueue>,
}

#[derive(SystemParam)]
pub struct RenderPlanAssemblyInputs<'w, 's> {
    outputs: Query<'w, 's, OutputRuntime>,
    scene_contributions: Res<'w, RenderSceneContributionQueue>,
    identity_registry: ResMut<'w, RenderSceneIdentityRegistry>,
    render_plan: ResMut<'w, RenderPlan>,
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
        surface_presentation,
        surface_visual,
        animation_timelines,
        mut scene_contributions,
    } = composition;
    let surface_presentation = surface_presentation.as_deref();
    let surface_visual = surface_visual.as_deref();
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
            Some((
                output_id,
                (window.surface_id(), surface_opacity(window.surface_id(), surface_visual)),
            ))
        })
        .fold(BTreeMap::new(), |mut backgrounds, (output_id, candidate)| {
            backgrounds
                .entry(output_id)
                .and_modify(|current: &mut (u64, f32)| {
                    if candidate.0 > current.0 {
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
                surface_opacity(window.surface_id(), surface_visual),
            ))
        })
        .collect::<Vec<_>>();
    let active_window_entities =
        visible_windows.iter().map(|(entity, ..)| *entity).collect::<BTreeSet<_>>();
    let active_window_opacity = visible_windows
        .iter()
        .map(|(_, surface_id, _, opacity)| (*surface_id, *opacity))
        .collect::<BTreeMap<_, _>>();
    let ordered_window_surfaces = stacking.ordered_surfaces(
        visible_windows.iter().map(|(_, surface_id, workspace_id, _)| (*workspace_id, *surface_id)),
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
                .map(|layer| {
                    (layer.surface_id(), surface_opacity(layer.surface_id(), surface_visual))
                }),
        )
        .chain(ordered_window_surfaces.into_iter().filter_map(|surface_id| {
            active_window_opacity.get(&surface_id).copied().map(|opacity| (surface_id, opacity))
        }))
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
                .map(|popup| {
                    (popup.surface_id(), surface_opacity(popup.surface_id(), surface_visual))
                }),
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
                .map(|layer| {
                    (layer.surface_id(), surface_opacity(layer.surface_id(), surface_visual))
                }),
        )
        .collect::<Vec<_>>();

    for (z_index, (surface_id, opacity)) in elements.into_iter().enumerate() {
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
            let visual_state = surface_visual_state(surface_id, surface_visual);
            let mut instance = RenderItemInstance {
                rect: visual_state
                    .as_ref()
                    .and_then(|state| state.rect_override)
                    .unwrap_or_else(|| RenderRect::from(&state.geometry)),
                opacity,
                clip_rect: visual_state.as_ref().and_then(|state| state.clip_rect_override),
                z_index: z_index as i32,
                scene_role: RenderSceneRole::Desktop,
            };
            apply_instance_animation_overrides(
                &mut instance,
                &animation_timelines,
                &RenderInstanceKey::new(RenderSourceKey::surface(surface_id), output_id, 0),
            );
            contributions
                .entry(output_id)
                .or_default()
                .push(RenderSceneContribution::surface(output_id, surface_id, 0, instance));
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

fn popup_parent_visible(child_of: &ChildOf, active_window_entities: &BTreeSet<Entity>) -> bool {
    active_window_entities.contains(&child_of.parent())
}

fn surface_opacity(surface_id: u64, visual_snapshot: Option<&SurfaceVisualSnapshot>) -> f32 {
    surface_visual_state(surface_id, visual_snapshot).map(|state| state.opacity).unwrap_or(1.0)
}

fn surface_visual_state(
    surface_id: u64,
    visual_snapshot: Option<&SurfaceVisualSnapshot>,
) -> Option<nekoland_ecs::resources::SurfaceVisualState> {
    visual_snapshot.and_then(|snapshot| snapshot.surfaces.get(&surface_id).cloned())
}

fn apply_instance_animation_overrides(
    instance: &mut RenderItemInstance,
    timelines: &AnimationTimelineStore,
    binding: &RenderInstanceKey,
) {
    let binding = AnimationBindingKey::Instance(binding.clone());
    if let Some(AnimationValue::Float(opacity)) =
        timelines.sampled_value(&binding, AnimationProperty::Opacity)
    {
        instance.opacity = (*opacity).clamp(0.0, 1.0);
    }
    if let Some(AnimationValue::Rect(rect)) =
        timelines.sampled_value(&binding, AnimationProperty::Rect)
    {
        instance.rect = *rect;
    }
    if let Some(AnimationValue::Rect(rect)) =
        timelines.sampled_value(&binding, AnimationProperty::ClipRect)
    {
        instance.clip_rect = Some(*rect);
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::{LayerSurfaceBundle, OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        LayerLevel, OutputBackgroundWindow, OutputDevice, OutputKind, OutputProperties,
        WindowAnimation, WindowRole, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{
        RenderColor, RenderItemIdentity, RenderItemInstance, RenderPlan, RenderPlanItem,
        RenderRect, RenderSceneRole, RenderSourceId, SurfacePresentationRole,
        SurfacePresentationSnapshot, SurfacePresentationState, SurfaceVisualSnapshot,
        SurfaceVisualState, UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
    };

    use crate::animation::{
        AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
        AnimationTrack, AnimationValue,
    };
    use crate::scene_source::{
        ExternalSceneContributionState, RenderInstanceKey, RenderSceneContribution,
        RenderSceneContributionPayload, RenderSceneContributionQueue, RenderSourceKey,
    };

    use super::{assemble_render_plan_system, emit_desktop_scene_contributions_system};

    fn add_render_plan_systems(app: &mut NekolandApp) {
        app.inner_mut()
            .init_resource::<crate::animation::AnimationTimelineStore>()
            .init_resource::<RenderSceneContributionQueue>()
            .init_resource::<ExternalSceneContributionState>()
            .init_resource::<crate::scene_source::RenderSceneIdentityRegistry>()
            .add_systems(
                RenderSchedule,
                (
                    crate::scene_source::clear_scene_contributions_system,
                    emit_desktop_scene_contributions_system,
                    crate::scene_source::emit_external_scene_contributions_system,
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
                RenderPlanItem::SolidRect(_)
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
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "front".to_owned(),
                last_acked_configure: None,
            },
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "back".to_owned(),
                last_acked_configure: None,
            },
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
                    last_acked_configure: None,
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
                },
            },
        ));
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            window: XdgWindow {
                app_id: "org.nekoland.test".to_owned(),
                title: "front".to_owned(),
                last_acked_configure: None,
            },
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
                    last_acked_configure: None,
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
    fn render_uses_surface_visual_snapshot_instead_of_animation_component() {
        let mut app = NekolandApp::new("render-surface-visual-boundary-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .init_resource::<SurfaceVisualSnapshot>()
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
                last_acked_configure: None,
            },
            animation: WindowAnimation { progress: 0.9, ..Default::default() },
            ..Default::default()
        });
        app.inner_mut()
            .world_mut()
            .resource_mut::<SurfaceVisualSnapshot>()
            .surfaces
            .insert(11, SurfaceVisualState { opacity: 0.25, ..Default::default() });
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
                last_acked_configure: None,
            },
            ..Default::default()
        });
        set_surface_states(&mut app, [(11, SurfacePresentationRole::Window)]);
        app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>().upsert_track(
            AnimationBindingKey::Instance(RenderInstanceKey::new(
                RenderSourceKey::surface(11),
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
                RenderSourceKey::surface(11),
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
                        last_acked_configure: None,
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
        app.inner_mut().world_mut().resource_mut::<ExternalSceneContributionState>().outputs =
            std::collections::BTreeMap::from([(
                output_id,
                vec![RenderSceneContribution {
                    key: RenderInstanceKey::new(
                        RenderSourceKey::new("test", "solid_rect"),
                        output_id,
                        0,
                    ),
                    payload: RenderSceneContributionPayload::SolidRect {
                        color: RenderColor { r: 10, g: 20, b: 30, a: 200 },
                    },
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 5, y: 6, width: 40, height: 50 },
                        opacity: 0.75,
                        clip_rect: None,
                        z_index: 3,
                        scene_role: RenderSceneRole::Overlay,
                    },
                }],
            )]);

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let output_plan = app
            .inner()
            .world()
            .resource::<RenderPlan>()
            .outputs
            .get(&output_id)
            .unwrap_or_else(|| panic!("output plan should exist for injected scene items"));
        assert!(matches!(output_plan.iter_ordered().next(), Some(RenderPlanItem::SolidRect(_))));
        let RenderPlanItem::SolidRect(item) =
            output_plan.iter_ordered().next().expect("expected one solid rect item")
        else {
            panic!("expected solid rect");
        };
        assert_eq!(item.identity, identity(1));
    }
}
