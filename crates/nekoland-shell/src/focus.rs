use std::collections::BTreeMap;

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Entity, Local, Query, Res, ResMut, With};
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::components::{OutputId, SurfaceGeometry, Window, WindowLayout, WindowMode};
use nekoland_ecs::events::PointerButton;
use nekoland_ecs::resources::{
    GlobalPointerPosition, KeyboardFocusState, OutputSnapshotState, UNASSIGNED_WORKSPACE_STACK_ID,
    ViewportAnimationActivityState, ViewportPointerPanState, WaylandIngress, WindowStackingState,
};
use nekoland_ecs::views::{WindowFocusRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::interaction::ActiveWindowGrab;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusManager;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusHoverState {
    initialized: bool,
    hovered_surface: Option<u64>,
}

type FocusWindows<'w, 's> = Query<'w, 's, WindowFocusRuntime, With<Window>>;
type FocusWorkspaces<'w, 's> = Query<'w, 's, (Entity, WorkspaceRuntime)>;

#[derive(SystemParam)]
pub struct FocusManagementParams<'w, 's> {
    active_grab: Option<Res<'w, ActiveWindowGrab>>,
    keyboard_focus: ResMut<'w, KeyboardFocusState>,
    wayland_ingress: Res<'w, WaylandIngress>,
    stacking: Res<'w, WindowStackingState>,
    viewport_animation: Option<Res<'w, ViewportAnimationActivityState>>,
    viewport_pan: Option<Res<'w, ViewportPointerPanState>>,
    windows: FocusWindows<'w, 's>,
    workspaces: FocusWorkspaces<'w, 's>,
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
    windows: Query<WindowFocusRuntime, With<Window>>,
    wayland_ingress: Res<WaylandIngress>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
) {
    if !pointer_buttons.read().any(|event| event.pressed) {
        return;
    }

    let visible_windows = visible_window_geometries(&windows, &workspaces);
    let output_context = pointer_output_context(&pointer, &wayland_ingress.output_snapshots);
    let ordered_surfaces = stacking.ordered_surfaces(
        visible_windows
            .iter()
            .map(|(surface_id, (_, workspace_id, _, _))| (*workspace_id, *surface_id)),
    );
    let focused_surface = wayland_ingress
        .pointer_focus_surface
        .filter(|surface_id| visible_windows.contains_key(surface_id))
        .or_else(|| {
            ordered_surfaces.iter().rev().find_map(|surface_id| {
                visible_windows
                    .get(surface_id)
                    .filter(|(geometry, _, output_name, _)| {
                        output_context.as_ref().map_or_else(
                            || pointer_in_geometry(pointer.x, pointer.y, geometry),
                            |(pointer_output, local_x, local_y)| {
                                output_name == &Some(*pointer_output)
                                    && pointer_in_geometry(*local_x, *local_y, geometry)
                            },
                        )
                    })
                    .map(|_| *surface_id)
            })
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
    mut hover_state: Local<FocusHoverState>,
    mut focus: FocusManagementParams<'_, '_>,
) {
    let visible_windows = visible_window_geometries(&focus.windows, &focus.workspaces);
    let output_context = pointer_output_context(&pointer, &focus.wayland_ingress.output_snapshots);
    let visible_surfaces = focus.stacking.ordered_surfaces(
        visible_windows
            .iter()
            .map(|(surface_id, (_, workspace_id, _, _))| (*workspace_id, *surface_id)),
    );

    if visible_surfaces.is_empty() {
        tracing::trace!(
            focused_surface = ?focus.keyboard_focus.focused_surface,
            "focus management skipped because no visible surfaces were derived"
        );
        return;
    }

    if let Some(grabbed_surface) =
        focus.active_grab.and_then(|grab| grab.state.as_ref().map(|state| state.surface_id))
        && visible_surfaces.contains(&grabbed_surface)
    {
        hover_state.initialized = true;
        hover_state.hovered_surface = Some(grabbed_surface);
        focus.keyboard_focus.focused_surface = Some(grabbed_surface);
        tracing::trace!(
            focused_surface = ?focus.keyboard_focus.focused_surface,
            "focus management tick"
        );
        return;
    }

    if config.focus_follows_mouse
        && !focus.viewport_pan.as_deref().is_some_and(|state| state.active)
        && !output_context.as_ref().is_some_and(|(output_id, _, _)| {
            focus
                .viewport_animation
                .as_deref()
                .is_some_and(|state| state.is_output_active(*output_id))
        })
    {
        let hovered_surface = visible_surfaces.iter().rev().find_map(|surface_id| {
            visible_windows
                .get(surface_id)
                .filter(|(geometry, _, output_name, layout)| {
                    *layout == WindowLayout::Tiled
                        && output_context.as_ref().map_or_else(
                            || pointer_in_geometry(pointer.x, pointer.y, geometry),
                            |(pointer_output, local_x, local_y)| {
                                output_name == &Some(*pointer_output)
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
                focus.keyboard_focus.focused_surface = Some(surface_id);
            } else if focus.keyboard_focus.focused_surface.is_none() {
                focus.keyboard_focus.focused_surface = hovered_surface;
            }
        } else if hovered_surface != hover_state.hovered_surface {
            if let Some(surface_id) = hovered_surface {
                focus.keyboard_focus.focused_surface = Some(surface_id);
            }
            hover_state.hovered_surface = hovered_surface;
        }
    }

    if focus
        .keyboard_focus
        .focused_surface
        .is_some_and(|surface_id| !visible_surfaces.contains(&surface_id))
    {
        focus.keyboard_focus.focused_surface = None;
    }

    if focus.keyboard_focus.focused_surface.is_none() {
        focus.keyboard_focus.focused_surface = visible_surfaces.last().copied();
    }

    tracing::trace!(focused_surface = ?focus.keyboard_focus.focused_surface, "focus management tick");
}

fn visible_window_geometries(
    windows: &Query<WindowFocusRuntime, With<Window>>,
    workspaces: &Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
) -> BTreeMap<u64, (SurfaceGeometry, u32, Option<OutputId>, WindowLayout)> {
    windows
        .iter()
        .filter_map(|window| {
            (*window.mode != WindowMode::Hidden
                && window.viewport_visibility.visible
                && window.role.is_managed()
                && !window.management_hints.helper_surface)
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
    outputs: &OutputSnapshotState,
) -> Option<(OutputId, f64, f64)> {
    outputs.outputs.iter().find_map(|output| {
        let left = f64::from(output.x);
        let top = f64::from(output.y);
        let right = left + f64::from(output.width.max(1));
        let bottom = top + f64::from(output.height.max(1));
        (pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom)
            .then(|| (output.output_id, pointer.x - left, pointer.y - top))
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
    use nekoland_config::resources::CompositorConfig;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        OutputId, SurfaceGeometry, WindowLayout, WindowViewportVisibility, WlSurfaceHandle,
    };
    use nekoland_ecs::events::PointerButton;
    use nekoland_ecs::resources::{
        GlobalPointerPosition, KeyboardFocusState, UNASSIGNED_WORKSPACE_STACK_ID,
        ViewportAnimationActivityState, ViewportPointerPanState, WaylandIngress,
        WindowStackingState,
    };

    use super::{focus_management_system, pointer_button_focus_system};

    #[test]
    fn clicking_visible_lower_window_raises_and_focuses_it() {
        let mut app = NekolandApp::new("focus-click-stack-test");
        app.inner_mut()
            .insert_resource(WaylandIngress::default())
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
        let config = CompositorConfig { focus_follows_mouse: false, ..CompositorConfig::default() };
        app.inner_mut()
            .insert_resource(WaylandIngress::default())
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
            .insert_resource(WaylandIngress::default())
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
    fn clicking_uses_wayland_pointer_focus_surface_when_available() {
        let mut app = NekolandApp::new("focus-click-wayland-pointer-focus-test");
        app.inner_mut()
            .insert_resource(WaylandIngress {
                pointer_focus_surface: Some(22),
                ..WaylandIngress::default()
            })
            .insert_resource(GlobalPointerPosition { x: 130.0, y: 10.0 })
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
            geometry: SurfaceGeometry { x: 120, y: 0, width: 120, height: 120 },
            layout: WindowLayout::Floating,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 0, y: 0, width: 100, height: 100 },
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
            .insert_resource(WaylandIngress::default())
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
            .insert_resource(WaylandIngress::default())
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
            .insert_resource(WaylandIngress::default())
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
            .insert_resource(WaylandIngress::default())
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

    #[test]
    fn viewport_animation_blocks_hover_focus_changes_on_pointer_output() {
        let output_id = OutputId(7);
        let mut app = NekolandApp::new("focus-hover-viewport-animation-test");
        app.inner_mut()
            .insert_resource(WaylandIngress {
                output_snapshots: nekoland_ecs::resources::OutputSnapshotState {
                    outputs: vec![nekoland_ecs::resources::OutputGeometrySnapshot {
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
                ..WaylandIngress::default()
            })
            .insert_resource(CompositorConfig::default())
            .insert_resource(GlobalPointerPosition { x: 10.0, y: 10.0 })
            .insert_resource(KeyboardFocusState { focused_surface: Some(22) })
            .insert_resource(ViewportAnimationActivityState {
                active_outputs: std::collections::BTreeSet::from([output_id]),
            })
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
            viewport_visibility: WindowViewportVisibility {
                visible: true,
                output: Some(output_id),
            },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 22 },
            geometry: SurfaceGeometry { x: 40, y: 40, width: 120, height: 120 },
            viewport_visibility: WindowViewportVisibility {
                visible: true,
                output: Some(output_id),
            },
            layout: WindowLayout::Tiled,
            ..Default::default()
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let focus = app.inner().world().resource::<KeyboardFocusState>();
        assert_eq!(focus.focused_surface, Some(22));
    }
}
