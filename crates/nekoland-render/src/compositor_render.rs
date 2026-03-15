use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Query, ResMut, With};
use nekoland_ecs::components::{LayerLevel, WindowMode, XdgPopup, XdgWindow};
use nekoland_ecs::resources::{
    RenderElement, RenderList, UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
};
use nekoland_ecs::views::{
    LayerRenderRuntime, PopupRenderRuntime, WindowRenderRuntime, WorkspaceRuntime,
};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameComposer;

/// Builds the per-frame render list from already-laid-out surfaces.
///
/// Composition order is deliberate: output backgrounds, background/bottom layers, visible
/// windows, popups whose parents are still visible, then top/overlay layers.
pub fn compose_frame_system(
    layers: Query<LayerRenderRuntime, With<nekoland_ecs::components::LayerShellSurface>>,
    windows: Query<(Entity, WindowRenderRuntime), With<XdgWindow>>,
    popups: Query<PopupRenderRuntime, With<XdgPopup>>,
    stacking: bevy_ecs::prelude::Res<WindowStackingState>,
    workspaces: Query<(Entity, WorkspaceRuntime)>,
    mut render_list: ResMut<RenderList>,
) {
    let background_windows = windows
        .iter()
        .filter(|(_, window)| {
            *window.mode != WindowMode::Hidden
                && window.viewport_visibility.visible
                && window.background.is_some()
        })
        .fold(BTreeMap::new(), |mut backgrounds, (_, window)| {
            let Some(background) = window.background.as_ref() else {
                return backgrounds;
            };
            let candidate = (window.surface_id(), opacity_for_animation(window.animation.progress));
            backgrounds
                .entry(background.output.clone())
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
        .filter(|(_, window)| {
            *window.mode != WindowMode::Hidden
                && window.viewport_visibility.visible
                && window.background.is_none()
        })
        .map(|(entity, window)| {
            (
                entity,
                window.surface_id(),
                window_workspace_runtime_id(window.child_of, &workspaces)
                    .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
                opacity_for_animation(window.animation.progress),
            )
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
    let mut elements = background_windows
        .into_iter()
        .chain(
            layers
                .iter()
                .filter(|layer| layer.buffer.attached)
                .filter(|layer| {
                    matches!(layer.layer_surface.layer, LayerLevel::Background | LayerLevel::Bottom)
                })
                .map(|layer| (layer.surface_id(), opacity_for_animation(layer.animation.progress))),
        )
        .chain(ordered_window_surfaces.into_iter().filter_map(|surface_id| {
            active_window_opacity.get(&surface_id).copied().map(|opacity| (surface_id, opacity))
        }))
        .chain(
            popups
                .iter()
                .filter(|popup| popup_parent_visible(popup.child_of, &active_window_entities))
                .map(|popup| (popup.surface_id(), opacity_for_animation(popup.animation.progress))),
        )
        .chain(
            layers
                .iter()
                .filter(|layer| {
                    layer.buffer.attached
                        && matches!(
                            layer.layer_surface.layer,
                            LayerLevel::Top | LayerLevel::Overlay
                        )
                })
                .map(|layer| (layer.surface_id(), opacity_for_animation(layer.animation.progress))),
        )
        .enumerate()
        .map(|(z_index, (surface_id, opacity))| RenderElement {
            surface_id,
            z_index: z_index as i32,
            opacity,
        })
        .collect::<Vec<_>>();

    elements.sort_by_key(|element| element.z_index);
    render_list.elements = elements;

    tracing::trace!(elements = render_list.elements.len(), "frame composition tick");
}

fn popup_parent_visible(child_of: &ChildOf, active_window_entities: &BTreeSet<Entity>) -> bool {
    active_window_entities.contains(&child_of.parent())
}

fn opacity_for_animation(animation_progress: f32) -> f32 {
    if animation_progress == 0.0 { 1.0 } else { animation_progress }
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::{LayerSurfaceBundle, WindowBundle};
    use nekoland_ecs::components::{
        LayerLevel, OutputBackgroundWindow, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::resources::{RenderList, UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState};

    use super::compose_frame_system;

    #[test]
    fn render_order_follows_window_stacking_state() {
        let mut app = NekolandApp::new("render-stack-order-test");
        app.inner_mut()
            .init_resource::<RenderList>()
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

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_list = &app.inner().world().resource::<RenderList>().elements;
        let render_order = render_list
            .iter()
            .filter(|element| element.surface_id != 0)
            .map(|element| element.surface_id)
            .collect::<Vec<_>>();
        assert_eq!(render_order, vec![22, 11]);
    }

    #[test]
    fn background_windows_render_below_normal_windows() {
        let mut app = NekolandApp::new("render-background-order-test");
        app.inner_mut()
            .init_resource::<RenderList>()
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
            OutputBackgroundWindow {
                output: "Virtual-1".to_owned(),
                restore: nekoland_ecs::components::WindowRestoreState {
                    geometry: Default::default(),
                    layout: nekoland_ecs::components::WindowLayout::Floating,
                    mode: nekoland_ecs::components::WindowMode::Normal,
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

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = app
            .inner()
            .world()
            .resource::<RenderList>()
            .elements
            .iter()
            .filter(|element| element.surface_id != 0)
            .map(|element| element.surface_id)
            .collect::<Vec<_>>();
        assert_eq!(render_order, vec![11, 22]);
    }

    #[test]
    fn background_windows_render_below_background_layers() {
        let mut app = NekolandApp::new("render-background-layer-order-test");
        app.inner_mut()
            .init_resource::<RenderList>()
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
            OutputBackgroundWindow {
                output: "Virtual-1".to_owned(),
                restore: nekoland_ecs::components::WindowRestoreState {
                    geometry: Default::default(),
                    layout: nekoland_ecs::components::WindowLayout::Floating,
                    mode: nekoland_ecs::components::WindowMode::Normal,
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

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = app
            .inner()
            .world()
            .resource::<RenderList>()
            .elements
            .iter()
            .filter(|element| element.surface_id != 0)
            .map(|element| element.surface_id)
            .collect::<Vec<_>>();
        assert_eq!(render_order, vec![11, 22]);
    }

    #[test]
    fn only_latest_background_window_per_output_renders() {
        let mut app = NekolandApp::new("render-single-background-per-output-test");
        app.inner_mut()
            .init_resource::<RenderList>()
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
                OutputBackgroundWindow {
                    output: "Virtual-1".to_owned(),
                    restore: nekoland_ecs::components::WindowRestoreState {
                        geometry: Default::default(),
                        layout: nekoland_ecs::components::WindowLayout::Floating,
                        mode: nekoland_ecs::components::WindowMode::Normal,
                    },
                },
            ));
        }

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let render_order = app
            .inner()
            .world()
            .resource::<RenderList>()
            .elements
            .iter()
            .filter(|element| element.surface_id != 0)
            .map(|element| element.surface_id)
            .collect::<Vec<_>>();
        assert_eq!(render_order, vec![22]);
    }
}
