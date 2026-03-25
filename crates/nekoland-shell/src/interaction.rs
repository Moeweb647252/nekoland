use bevy_ecs::message::{MessageReader, MessageWriter};
use bevy_ecs::prelude::{Commands, Query, Res, ResMut, Resource, With};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{
    PendingInteractiveResize, Window, WindowLayout, WindowPosition, WindowSceneGeometry, WindowSize,
};
use nekoland_ecs::events::{PointerButton, WindowMoved};
use nekoland_ecs::resources::{
    EntityIndex, GlobalPointerPosition, KeyboardFocusState, ResizeEdges,
    UNASSIGNED_WORKSPACE_STACK_ID, WaylandIngress, WindowStackingState,
};
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::{
    preferred_primary_output_id, project_scene_geometry, resolve_output_state_for_workspace,
};

const MIN_WINDOW_SIZE: i32 = 32;

type GrabWindows<'w, 's> = Query<'w, 's, WindowRuntime, With<Window>>;
type GrabOutputs<'w, 's> = Query<'w, 's, (bevy_ecs::prelude::Entity, OutputRuntime)>;
type GrabWorkspaces<'w, 's> = Query<'w, 's, (bevy_ecs::prelude::Entity, WorkspaceRuntime)>;

#[derive(SystemParam)]
pub struct WindowGrabParams<'w, 's> {
    entity_index: Res<'w, EntityIndex>,
    pointer: Res<'w, GlobalPointerPosition>,
    active_grab: ResMut<'w, ActiveWindowGrab>,
    keyboard_focus: ResMut<'w, KeyboardFocusState>,
    stacking: ResMut<'w, WindowStackingState>,
    wayland_ingress: Res<'w, WaylandIngress>,
    window_moved: MessageWriter<'w, WindowMoved>,
    windows: GrabWindows<'w, 's>,
    outputs: GrabOutputs<'w, 's>,
    workspaces: GrabWorkspaces<'w, 's>,
}

/// Interactive grab mode currently applied to a floating window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowGrabMode {
    Move,
    Resize { edges: ResizeEdges },
}

/// Snapshot captured when an interactive move/resize grab begins.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowGrabState {
    pub surface_id: u64,
    pub mode: WindowGrabMode,
    pub start_pointer_x: f64,
    pub start_pointer_y: f64,
    pub start_scene_geometry: WindowSceneGeometry,
}

/// Current interactive window grab, if any.
#[derive(Debug, Clone, Default, PartialEq, Resource)]
pub struct ActiveWindowGrab {
    pub state: Option<WindowGrabState>,
}

/// Advances the active floating-window grab from pointer motion/button events and applies the
/// resulting geometry updates in real time.
pub fn window_grab_system(
    mut commands: Commands,
    mut pointer_buttons: MessageReader<PointerButton>,
    mut grab: WindowGrabParams<'_, '_>,
) {
    let release_detected = pointer_buttons.read().any(|event| !event.pressed);
    let Some(grab_state) = grab.active_grab.state.clone() else {
        return;
    };

    let Some(entity) = grab.entity_index.entity_for_surface(grab_state.surface_id) else {
        grab.active_grab.state = None;
        return;
    };
    let Some(mut window) = grab.windows.get_mut(entity).ok() else {
        grab.active_grab.state = None;
        return;
    };

    if *window.layout != WindowLayout::Floating {
        grab.active_grab.state = None;
        return;
    }

    let resize_grab = matches!(grab_state.mode, WindowGrabMode::Resize { .. });
    let next_geometry =
        geometry_for_pointer(&grab_state, &grab.pointer, Some(&window.scene_geometry));
    let moved =
        window.scene_geometry.x != next_geometry.x || window.scene_geometry.y != next_geometry.y;
    let resized = window.scene_geometry.width != next_geometry.width
        || window.scene_geometry.height != next_geometry.height;
    let defer_native_xdg_resize =
        resize_grab && resized && window.management_hints.client_driven_resize;

    if defer_native_xdg_resize {
        let WindowGrabMode::Resize { edges } = grab_state.mode else { return };
        if let Some(mut pending_resize) = window.pending_resize {
            pending_resize.requested_geometry = next_geometry.clone();
            pending_resize.edges = edges;
        } else {
            commands.entity(entity).insert(PendingInteractiveResize {
                requested_geometry: next_geometry.clone(),
                inflight_geometry: None,
                edges,
            });
        }
    } else if moved || resized {
        *window.scene_geometry = next_geometry.clone();
        let workspace_id = window_workspace_runtime_id(window.child_of, &grab.workspaces)
            .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID);
        let primary_output_id = preferred_primary_output_id(Some(&grab.wayland_ingress));
        if let Some((_, _, viewport, _)) =
            resolve_output_state_for_workspace(&grab.outputs, Some(workspace_id), primary_output_id)
        {
            *window.geometry = project_scene_geometry(&next_geometry, viewport);
        } else {
            window.geometry.x = next_geometry.x.clamp(i32::MIN as isize, i32::MAX as isize) as i32;
            window.geometry.y = next_geometry.y.clamp(i32::MIN as isize, i32::MAX as isize) as i32;
            window.geometry.width = next_geometry.width;
            window.geometry.height = next_geometry.height;
        }
        if moved {
            window
                .placement
                .set_explicit_position(WindowPosition { x: next_geometry.x, y: next_geometry.y });
            grab.window_moved.write(WindowMoved {
                surface_id: grab_state.surface_id,
                x: next_geometry.x as i64,
                y: next_geometry.y as i64,
            });
        }
        if resized {
            window.placement.floating_size =
                Some(WindowSize { width: next_geometry.width, height: next_geometry.height });
            if !moved {
                window.placement.set_explicit_position(WindowPosition {
                    x: next_geometry.x,
                    y: next_geometry.y,
                });
            }
        }
    }

    grab.keyboard_focus.focused_surface = Some(grab_state.surface_id);
    grab.stacking.raise(
        window_workspace_runtime_id(window.child_of, &grab.workspaces)
            .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
        grab_state.surface_id,
    );

    if release_detected {
        grab.active_grab.state = None;
    }
}

/// Starts a new interactive window grab using the current pointer position and geometry as the
/// reference frame.
pub(crate) fn begin_window_grab(
    active_grab: &mut ActiveWindowGrab,
    surface_id: u64,
    mode: WindowGrabMode,
    pointer: &GlobalPointerPosition,
    geometry: &WindowSceneGeometry,
) {
    active_grab.state = Some(WindowGrabState {
        surface_id,
        mode,
        start_pointer_x: pointer.x,
        start_pointer_y: pointer.y,
        start_scene_geometry: geometry.clone(),
    });
}

/// Computes the geometry implied by the current pointer position for an active grab.
fn geometry_for_pointer(
    grab_state: &WindowGrabState,
    pointer: &GlobalPointerPosition,
    current_geometry: Option<&WindowSceneGeometry>,
) -> WindowSceneGeometry {
    let delta_x = (pointer.x - grab_state.start_pointer_x).round() as i32;
    let delta_y = (pointer.y - grab_state.start_pointer_y).round() as i32;

    match &grab_state.mode {
        WindowGrabMode::Move => WindowSceneGeometry {
            x: grab_state.start_scene_geometry.x.saturating_add(delta_x as isize),
            y: grab_state.start_scene_geometry.y.saturating_add(delta_y as isize),
            width: current_geometry
                .map(|geometry| geometry.width)
                .unwrap_or(grab_state.start_scene_geometry.width),
            height: current_geometry
                .map(|geometry| geometry.height)
                .unwrap_or(grab_state.start_scene_geometry.height),
        },
        WindowGrabMode::Resize { edges } => {
            resize_geometry(&grab_state.start_scene_geometry, *edges, delta_x, delta_y)
        }
    }
}

/// Applies edge-specific resize semantics while enforcing a minimum window size.
fn resize_geometry(
    start_geometry: &WindowSceneGeometry,
    edges: ResizeEdges,
    delta_x: i32,
    delta_y: i32,
) -> WindowSceneGeometry {
    let mut x = start_geometry.x;
    let mut y = start_geometry.y;
    let mut width = start_geometry.width as i32;
    let mut height = start_geometry.height as i32;

    if edges.has_left() {
        let desired_width = width - delta_x;
        if desired_width < MIN_WINDOW_SIZE {
            x = x.saturating_add((width - MIN_WINDOW_SIZE) as isize);
            width = MIN_WINDOW_SIZE;
        } else {
            x = x.saturating_add(delta_x as isize);
            width = desired_width;
        }
    }
    if edges.has_right() {
        width = (width + delta_x).max(MIN_WINDOW_SIZE);
    }
    if edges.has_top() {
        let desired_height = height - delta_y;
        if desired_height < MIN_WINDOW_SIZE {
            y = y.saturating_add((height - MIN_WINDOW_SIZE) as isize);
            height = MIN_WINDOW_SIZE;
        } else {
            y = y.saturating_add(delta_y as isize);
            height = desired_height;
        }
    }
    if edges.has_bottom() {
        height = (height + delta_y).max(MIN_WINDOW_SIZE);
    }

    WindowSceneGeometry {
        x,
        y,
        width: width.max(MIN_WINDOW_SIZE) as u32,
        height: height.max(MIN_WINDOW_SIZE) as u32,
    }
}

#[cfg(test)]
mod tests {
    use nekoland_ecs::components::WindowSceneGeometry;
    use nekoland_ecs::resources::GlobalPointerPosition;
    use nekoland_ecs::resources::ResizeEdges;

    use super::{WindowGrabMode, WindowGrabState, geometry_for_pointer};

    fn grab_state(mode: WindowGrabMode) -> WindowGrabState {
        WindowGrabState {
            surface_id: 7,
            mode,
            start_pointer_x: 100.0,
            start_pointer_y: 200.0,
            start_scene_geometry: WindowSceneGeometry { x: 40, y: 60, width: 800, height: 600 },
        }
    }

    #[test]
    fn move_grab_tracks_pointer_delta() {
        let geometry = geometry_for_pointer(
            &grab_state(WindowGrabMode::Move),
            &GlobalPointerPosition { x: 148.0, y: 236.0 },
            None,
        );

        assert_eq!(geometry.x, 88);
        assert_eq!(geometry.y, 96);
        assert_eq!(geometry.width, 800);
        assert_eq!(geometry.height, 600);
    }

    #[test]
    fn resize_grab_respects_minimum_size_on_left_edge() {
        let geometry = geometry_for_pointer(
            &grab_state(WindowGrabMode::Resize { edges: ResizeEdges::Left }),
            &GlobalPointerPosition { x: 900.0, y: 200.0 },
            None,
        );

        assert_eq!(geometry.x, 808);
        assert_eq!(geometry.width, 32);
    }

    #[test]
    fn bottom_right_resize_expands_geometry_by_pointer_delta() {
        let geometry = geometry_for_pointer(
            &grab_state(WindowGrabMode::Resize { edges: ResizeEdges::BottomRight }),
            &GlobalPointerPosition { x: 164.0, y: 248.0 },
            None,
        );

        assert_eq!(geometry.width, 864);
        assert_eq!(geometry.height, 648);
    }

    #[test]
    fn move_grab_preserves_current_size_when_client_size_changed_during_grab() {
        let geometry = geometry_for_pointer(
            &grab_state(WindowGrabMode::Move),
            &GlobalPointerPosition { x: 148.0, y: 236.0 },
            Some(&WindowSceneGeometry { x: 40, y: 60, width: 960, height: 720 }),
        );

        assert_eq!(geometry.x, 88);
        assert_eq!(geometry.y, 96);
        assert_eq!(geometry.width, 960);
        assert_eq!(geometry.height, 720);
    }
}
