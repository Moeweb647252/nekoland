use std::collections::BTreeMap;

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Local, Query, Res, ResMut, With};
use nekoland_ecs::components::{SurfaceGeometry, WindowLayout, WindowMode, XdgWindow};
use nekoland_ecs::events::PointerButton;
use nekoland_ecs::resources::{
    CompositorConfig, GlobalPointerPosition, KeyboardFocusState, UNASSIGNED_WORKSPACE_STACK_ID,
    ViewportPointerPanState, WindowStackingState,
};
use nekoland_ecs::views::{OutputRuntime, WindowFocusRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::interaction::ActiveWindowGrab;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusManager;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusHoverState {
    initialized: bool,
    hovered_surface: Option<u64>,
}

/// Maintains keyboard focus from the current visible window stack.
///
/// Active grabs win first, then optional tiled-only focus-follows-mouse hover, and finally a
/// fallback to the front-most visible window if the previous focus disappeared.
pub fn pointer_button_focus_system(
    pointer: Res<GlobalPointerPosition>,
    mut pointer_buttons: MessageReader<PointerButton>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut stacking: ResMut<WindowStackingState>,
    windows: Query<WindowFocusRuntime, With<XdgWindow>>,
    outputs: Query<OutputRuntime>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
) {
    if !pointer_buttons.read().any(|event| event.pressed) {
        return;
    }

    let visible_windows = visible_window_geometries(&windows, &workspaces);
    let output_context = pointer_output_context(&pointer, &outputs);
    let ordered_surfaces = stacking.ordered_surfaces(
        visible_windows
            .iter()
            .map(|(surface_id, (_, workspace_id, _, _))| (*workspace_id, *surface_id)),
    );
    let focused_surface = ordered_surfaces.iter().rev().find_map(|surface_id| {
        visible_windows
            .get(surface_id)
            .filter(|(geometry, _, output_name, _)| {
                output_context.as_ref().map_or_else(
                    || pointer_in_geometry(pointer.x, pointer.y, geometry),
                    |(pointer_output, local_x, local_y)| {
                        output_name.as_deref() == Some(pointer_output.as_str())
                            && pointer_in_geometry(*local_x, *local_y, geometry)
                    },
                )
            })
            .map(|_| *surface_id)
    });

    if let Some(surface_id) = focused_surface {
        keyboard_focus.focused_surface = Some(surface_id);
        if let Some((_, workspace_id, _, _)) = visible_windows.get(&surface_id) {
            stacking.raise(*workspace_id, surface_id);
        }
    }
}

pub fn focus_management_system(
    config: Res<CompositorConfig>,
    pointer: Res<GlobalPointerPosition>,
    active_grab: Option<Res<ActiveWindowGrab>>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    stacking: Res<WindowStackingState>,
    viewport_pan: Option<Res<ViewportPointerPanState>>,
    mut hover_state: Local<FocusHoverState>,
    windows: Query<WindowFocusRuntime, With<XdgWindow>>,
    outputs: Query<OutputRuntime>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
) {
    let visible_windows = visible_window_geometries(&windows, &workspaces);
    let output_context = pointer_output_context(&pointer, &outputs);
    let visible_surfaces = stacking.ordered_surfaces(
        visible_windows
            .iter()
            .map(|(surface_id, (_, workspace_id, _, _))| (*workspace_id, *surface_id)),
    );

    if let Some(grabbed_surface) =
        active_grab.and_then(|grab| grab.state.as_ref().map(|state| state.surface_id))
    {
        if visible_surfaces.contains(&grabbed_surface) {
            hover_state.initialized = true;
            hover_state.hovered_surface = Some(grabbed_surface);
            keyboard_focus.focused_surface = Some(grabbed_surface);
            tracing::trace!(focused_surface = ?keyboard_focus.focused_surface, "focus management tick");
            return;
        }
    }

    if config.focus_follows_mouse && !viewport_pan.as_deref().is_some_and(|state| state.active) {
        let hovered_surface = visible_surfaces.iter().rev().find_map(|surface_id| {
            visible_windows
                .get(surface_id)
                .filter(|(geometry, _, output_name, layout)| {
                    *layout == WindowLayout::Tiled
                        && output_context.as_ref().map_or_else(
                            || pointer_in_geometry(pointer.x, pointer.y, geometry),
                            |(pointer_output, local_x, local_y)| {
                                output_name.as_deref() == Some(pointer_output.as_str())
                                    && pointer_in_geometry(*local_x, *local_y, geometry)
                            },
                        )
                })
                .map(|_| *surface_id)
        });

        if !hover_state.initialized {
            hover_state.initialized = true;
            hover_state.hovered_surface = hovered_surface;
            if let Some(surface_id) = hovered_surface {
                keyboard_focus.focused_surface = Some(surface_id);
            } else if keyboard_focus.focused_surface.is_none() {
                keyboard_focus.focused_surface = hovered_surface;
            }
        } else if hovered_surface != hover_state.hovered_surface {
            if let Some(surface_id) = hovered_surface {
                keyboard_focus.focused_surface = Some(surface_id);
            }
            hover_state.hovered_surface = hovered_surface;
        }
    }

    if keyboard_focus
        .focused_surface
        .is_some_and(|surface_id| !visible_surfaces.contains(&surface_id))
    {
        keyboard_focus.focused_surface = None;
    }

    if keyboard_focus.focused_surface.is_none() {
        keyboard_focus.focused_surface = visible_surfaces.last().copied();
    }

    tracing::trace!(focused_surface = ?keyboard_focus.focused_surface, "focus management tick");
}

fn visible_window_geometries(
    windows: &Query<WindowFocusRuntime, With<XdgWindow>>,
    workspaces: &Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
) -> BTreeMap<u64, (SurfaceGeometry, u32, Option<String>, WindowLayout)> {
    windows
        .iter()
        .filter_map(|window| {
            (*window.mode != WindowMode::Hidden
                && window.viewport_visibility.visible
                && window.background.is_none()
                && window.x11_window.is_none_or(|x11_window| !x11_window.override_redirect))
            .then_some((
                window.surface_id(),
                (
                    window.geometry.clone(),
                    window_workspace_runtime_id(window.child_of, workspaces)
                        .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
                    window.viewport_visibility.output.clone(),
                    *window.layout,
                ),
            ))
        })
        .collect()
}

fn pointer_output_context(
    pointer: &GlobalPointerPosition,
    outputs: &Query<OutputRuntime>,
) -> Option<(String, f64, f64)> {
    outputs.iter().find_map(|output| {
        let left = f64::from(output.placement.x);
        let top = f64::from(output.placement.y);
        let right = left + f64::from(output.properties.width.max(1));
        let bottom = top + f64::from(output.properties.height.max(1));
        (pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom)
            .then(|| (output.name().to_owned(), pointer.x - left, pointer.y - top))
    })
}

/// Uses inclusive-left/top and exclusive-right/bottom bounds so adjacent windows do not both
/// claim the pointer when their edges touch.
fn pointer_in_geometry(pointer_x: f64, pointer_y: f64, geometry: &SurfaceGeometry) -> bool {
    let left = f64::from(geometry.x);
    let top = f64::from(geometry.y);
    let right = left + f64::from(geometry.width);
    let bottom = top + f64::from(geometry.height);

    pointer_x >= left && pointer_x < right && pointer_y >= top && pointer_y < bottom
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{SurfaceGeometry, WindowLayout, WlSurfaceHandle};
    use nekoland_ecs::events::PointerButton;
    use nekoland_ecs::resources::{
        CompositorConfig, GlobalPointerPosition, KeyboardFocusState, UNASSIGNED_WORKSPACE_STACK_ID,
        ViewportPointerPanState, WindowStackingState,
    };

    use super::{focus_management_system, pointer_button_focus_system};

    #[test]
    fn clicking_visible_lower_window_raises_and_focuses_it() {
        let mut app = NekolandApp::new("focus-click-stack-test");
        app.inner_mut()
            .insert_resource(GlobalPointerPosition { x: 10.0, y: 10.0 })
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_message::<PointerButton>()
            .add_systems(LayoutSchedule, pointer_button_focus_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });
        app.inner_mut()
            .world_mut()
            .write_message(PointerButton { button_code: 0x110, pressed: true });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let focus = world.resource::<KeyboardFocusState>();
        let stacking = world.resource::<WindowStackingState>();
        assert_eq!(focus.focused_surface, Some(11));
        assert_eq!(stacking.workspaces.get(&UNASSIGNED_WORKSPACE_STACK_ID), Some(&vec![22, 11]));
    }

    #[test]
    fn fallback_focus_uses_topmost_visible_window() {
        let mut app = NekolandApp::new("focus-fallback-stack-test");
        let mut config = CompositorConfig::default();
        config.focus_follows_mouse = false;
        app.inner_mut()
            .insert_resource(config)
            .insert_resource(GlobalPointerPosition::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_systems(LayoutSchedule, focus_management_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(22));
    }

    #[test]
    fn clicking_overlapping_windows_focuses_frontmost_window() {
        let mut app = NekolandApp::new("focus-click-overlap-test");
        app.inner_mut()
            .insert_resource(GlobalPointerPosition { x: 60.0, y: 60.0 })
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_message::<PointerButton>()
            .add_systems(LayoutSchedule, pointer_button_focus_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });
        app.inner_mut()
            .world_mut()
            .write_message(PointerButton { button_code: 0x110, pressed: true });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(22));
    }

    #[test]
    fn focus_follows_mouse_ignores_floating_windows() {
        let mut app = NekolandApp::new("focus-hover-floating-test");
        app.inner_mut()
            .insert_resource(CompositorConfig::default())
            .insert_resource(GlobalPointerPosition { x: 10.0, y: 10.0 })
            .insert_resource(KeyboardFocusState { focused_surface: Some(22) })
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_systems(LayoutSchedule, focus_management_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(22));
    }

    #[test]
    fn focus_follows_mouse_targets_tiled_windows() {
        let mut app = NekolandApp::new("focus-hover-tiled-test");
        app.inner_mut()
            .insert_resource(CompositorConfig::default())
            .insert_resource(GlobalPointerPosition { x: 10.0, y: 10.0 })
            .insert_resource(KeyboardFocusState { focused_surface: Some(22) })
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_systems(LayoutSchedule, focus_management_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(11));
    }

    #[test]
    fn focus_follows_mouse_prefers_frontmost_overlapping_tiled_window() {
        let mut app = NekolandApp::new("focus-hover-overlap-test");
        app.inner_mut()
            .insert_resource(CompositorConfig::default())
            .insert_resource(GlobalPointerPosition { x: 60.0, y: 60.0 })
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_systems(LayoutSchedule, focus_management_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(22));
    }

    #[test]
    fn viewport_pointer_pan_blocks_hover_focus_changes() {
        let mut app = NekolandApp::new("focus-hover-viewport-pan-test");
        app.inner_mut()
            .insert_resource(CompositorConfig::default())
            .insert_resource(GlobalPointerPosition { x: 10.0, y: 10.0 })
            .insert_resource(KeyboardFocusState { focused_surface: Some(22) })
            .insert_resource(ViewportPointerPanState { active: true })
            .insert_resource(WindowStackingState {
                workspaces: std::collections::BTreeMap::from([(
                    UNASSIGNED_WORKSPACE_STACK_ID,
                    vec![11, 22],
                )]),
            })
            .add_systems(LayoutSchedule, focus_management_system);

        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 11 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(22));
    }
}
