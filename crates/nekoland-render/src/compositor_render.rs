use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{XdgPopup, XdgWindow};
use nekoland_ecs::presentation_logic::{
    is_background_band_layer, is_foreground_band_layer, managed_window_visible, popup_visible,
};
use nekoland_ecs::resources::{
    OutputRenderPlan, RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole,
    SurfacePresentationRole, SurfacePresentationSnapshot, SurfaceRenderItem, SurfaceVisualSnapshot,
    UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
};
use nekoland_ecs::views::{
    LayerRenderRuntime, OutputRuntime, PopupRenderRuntime, WindowRenderRuntime, WorkspaceRuntime,
};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

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
    render_plan: ResMut<'w, RenderPlan>,
}

/// Builds the per-frame output-scoped render plan from already-laid-out surfaces plus projected
/// visual state.
///
/// Composition order is deliberate: output backgrounds, background/bottom layers, visible
/// windows, popups whose parents are still visible, then top/overlay layers.
pub fn compose_frame_system(composition: FrameCompositionInputs<'_, '_>) {
    let FrameCompositionInputs {
        outputs,
        layers,
        windows,
        popups,
        stacking,
        workspaces,
        surface_presentation,
        surface_visual,
        mut render_plan,
    } = composition;
    let surface_presentation = surface_presentation.as_deref();
    let surface_visual = surface_visual.as_deref();
    let live_outputs =
        outputs.iter().map(|output| (output.id(), output.name().to_owned())).collect::<Vec<_>>();
    let output_ids_by_name = live_outputs
        .iter()
        .map(|(output_id, output_name)| (output_name.clone(), *output_id))
        .collect::<BTreeMap<_, _>>();
    let mut plans = live_outputs
        .iter()
        .map(|(output_id, _)| (*output_id, OutputRenderPlan::default()))
        .collect::<BTreeMap<_, _>>();
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
            let output_id = state.and_then(|state| state.target_output.clone()).or_else(|| {
                window
                    .background
                    .as_ref()
                    .and_then(|background| output_ids_by_name.get(&background.output).copied())
            })?;
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
            live_outputs.iter().map(|(output_id, _)| *output_id).collect::<Vec<_>>()
        };

        for output_id in target_outputs {
            plans.entry(output_id).or_default().push(RenderPlanItem::Surface(SurfaceRenderItem {
                surface_id,
                rect: RenderRect::from(&state.geometry),
                opacity,
                z_index: z_index as i32,
                clip_rect: None,
                scene_role: RenderSceneRole::Desktop,
            }));
        }
    }

    for output_plan in plans.values_mut() {
        output_plan.sort_by_z_index();
    }
    render_plan.outputs = plans;

    tracing::trace!(
        outputs = render_plan.outputs.len(),
        elements = render_plan.outputs.values().map(|plan| plan.items.len()).sum::<usize>(),
        "frame composition tick"
    );
}

fn popup_parent_visible(child_of: &ChildOf, active_window_entities: &BTreeSet<Entity>) -> bool {
    active_window_entities.contains(&child_of.parent())
}

fn surface_opacity(surface_id: u64, visual_snapshot: Option<&SurfaceVisualSnapshot>) -> f32 {
    visual_snapshot
        .and_then(|snapshot| snapshot.surfaces.get(&surface_id))
        .map(|state| state.opacity)
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::{LayerSurfaceBundle, OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        LayerLevel, OutputBackgroundWindow, OutputDevice, OutputKind, OutputProperties,
        WindowAnimation, WindowRole, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{
        RenderPlan, RenderPlanItem, SurfacePresentationRole, SurfacePresentationSnapshot,
        SurfacePresentationState, SurfaceVisualSnapshot, SurfaceVisualState,
        UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
    };

    use super::compose_frame_system;

    fn spawn_default_output(app: &mut NekolandApp) {
        app.inner_mut().world_mut().spawn(OutputBundle {
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
        });
    }

    fn single_output_surface_order(app: &NekolandApp) -> Vec<u64> {
        app.inner()
            .world()
            .resource::<RenderPlan>()
            .outputs
            .values()
            .next()
            .into_iter()
            .flat_map(|plan| plan.items.iter())
            .filter_map(|item| match item {
                RenderPlanItem::Surface(item) => Some(item.surface_id),
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
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![22, 11],
                )]),
            })
            .add_systems(RenderSchedule, compose_frame_system);

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
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![22],
                )]),
            })
            .add_systems(RenderSchedule, compose_frame_system);

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
                output: "Virtual-1".to_owned(),
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
        spawn_default_output(&mut app);
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
            .insert_resource(WindowStackingState::default())
            .add_systems(RenderSchedule, compose_frame_system);

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
                output: "Virtual-1".to_owned(),
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
        spawn_default_output(&mut app);
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
            })
            .add_systems(RenderSchedule, compose_frame_system);

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
            .insert(11, SurfaceVisualState { opacity: 0.25 });
        spawn_default_output(&mut app);
        set_surface_states(&mut app, [(11, SurfacePresentationRole::Window)]);

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_plan = app.inner().world().resource::<RenderPlan>();
        let output_plan = render_plan.outputs.values().next().expect("single output render plan");
        assert_eq!(output_plan.items.len(), 1);
        let RenderPlanItem::Surface(item) = &output_plan.items[0];
        assert_eq!(item.opacity, 0.25);
    }

    #[test]
    fn only_latest_background_window_per_output_renders() {
        let mut app = NekolandApp::new("render-single-background-per-output-test");
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .insert_resource(WindowStackingState::default())
            .add_systems(RenderSchedule, compose_frame_system);

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
                    output: "Virtual-1".to_owned(),
                    restore: nekoland_ecs::components::WindowRestoreState {
                        geometry: Default::default(),
                        layout: nekoland_ecs::components::WindowLayout::Floating,
                        mode: nekoland_ecs::components::WindowMode::Normal,
                        fullscreen_output: None,
                    },
                },
            ));
        }
        spawn_default_output(&mut app);
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
}
