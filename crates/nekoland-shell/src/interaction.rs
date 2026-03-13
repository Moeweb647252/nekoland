use bevy_ecs::message::{MessageReader, MessageWriter};
use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use nekoland_ecs::components::{
    SurfaceGeometry, WindowLayout, WindowPosition, WindowSize, XdgWindow,
};
use nekoland_ecs::events::{PointerButton, WindowMoved};
use nekoland_ecs::resources::{
    EntityIndex, GlobalPointerPosition, KeyboardFocusState, ResizeEdges,
    UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState,
};
use nekoland_ecs::views::{WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

const MIN_WINDOW_SIZE: i32 = 64;

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
    pub start_geometry: SurfaceGeometry,
}

/// Current interactive window grab, if any.
#[derive(Debug, Clone, Default, PartialEq, Resource)]
pub struct ActiveWindowGrab {
    pub state: Option<WindowGrabState>,
}

/// Advances the active floating-window grab from pointer motion/button events and applies the
/// resulting geometry updates in real time.
pub fn window_grab_system(
    entity_index: Res<EntityIndex>,
    pointer: Res<GlobalPointerPosition>,
    mut pointer_buttons: MessageReader<PointerButton>,
    mut active_grab: ResMut<ActiveWindowGrab>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut stacking: ResMut<WindowStackingState>,
    mut windows: Query<WindowRuntime, With<XdgWindow>>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime)>,
    mut window_moved: MessageWriter<WindowMoved>,
) {
    let release_detected = pointer_buttons.read().any(|event| !event.pressed);
    let Some(grab_state) = active_grab.state.clone() else {
        return;
    };

    let Some(mut window) = entity_index
        .entity_for_surface(grab_state.surface_id)
        .and_then(|entity| windows.get_mut(entity).ok())
    else {
        active_grab.state = None;
        return;
    };

    if *window.layout != WindowLayout::Floating {
        active_grab.state = None;
        return;
    }

    let next_geometry = geometry_for_pointer(&grab_state, &pointer);
    let moved = window.geometry.x != next_geometry.x || window.geometry.y != next_geometry.y;
    let resized = window.geometry.width != next_geometry.width
        || window.geometry.height != next_geometry.height;

    if moved || resized {
        *window.geometry = next_geometry.clone();
        if moved {
            window
                .placement
                .set_explicit_position(WindowPosition { x: next_geometry.x, y: next_geometry.y });
            window_moved.write(WindowMoved {
                surface_id: grab_state.surface_id,
                x: next_geometry.x,
                y: next_geometry.y,
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

    keyboard_focus.focused_surface = Some(grab_state.surface_id);
    stacking.raise(
        window_workspace_runtime_id(window.child_of, &workspaces)
            .unwrap_or(UNASSIGNED_WORKSPACE_STACK_ID),
        grab_state.surface_id,
    );

    if release_detected {
        active_grab.state = None;
    }
}

/// Starts a new interactive window grab using the current pointer position and geometry as the
/// reference frame.
pub(crate) fn begin_window_grab(
    active_grab: &mut ActiveWindowGrab,
    surface_id: u64,
    mode: WindowGrabMode,
    pointer: &GlobalPointerPosition,
    geometry: &SurfaceGeometry,
) {
    active_grab.state = Some(WindowGrabState {
        surface_id,
        mode,
        start_pointer_x: pointer.x,
        start_pointer_y: pointer.y,
        start_geometry: geometry.clone(),
    });
}

/// Computes the geometry implied by the current pointer position for an active grab.
fn geometry_for_pointer(
    grab_state: &WindowGrabState,
    pointer: &GlobalPointerPosition,
) -> SurfaceGeometry {
    let delta_x = (pointer.x - grab_state.start_pointer_x).round() as i32;
    let delta_y = (pointer.y - grab_state.start_pointer_y).round() as i32;

    match &grab_state.mode {
        WindowGrabMode::Move => SurfaceGeometry {
            x: grab_state.start_geometry.x + delta_x,
            y: grab_state.start_geometry.y + delta_y,
            ..grab_state.start_geometry.clone()
        },
        WindowGrabMode::Resize { edges } => {
            resize_geometry(&grab_state.start_geometry, *edges, delta_x, delta_y)
        }
    }
}

/// Applies edge-specific resize semantics while enforcing a minimum window size.
fn resize_geometry(
    start_geometry: &SurfaceGeometry,
    edges: ResizeEdges,
    delta_x: i32,
    delta_y: i32,
) -> SurfaceGeometry {
    let mut x = start_geometry.x;
    let mut y = start_geometry.y;
    let mut width = start_geometry.width as i32;
    let mut height = start_geometry.height as i32;

    if edges.has_left() {
        let desired_width = width - delta_x;
        if desired_width < MIN_WINDOW_SIZE {
            x += width - MIN_WINDOW_SIZE;
            width = MIN_WINDOW_SIZE;
        } else {
            x += delta_x;
            width = desired_width;
        }
    }
    if edges.has_right() {
        width = (width + delta_x).max(MIN_WINDOW_SIZE);
    }
    if edges.has_top() {
        let desired_height = height - delta_y;
        if desired_height < MIN_WINDOW_SIZE {
            y += height - MIN_WINDOW_SIZE;
            height = MIN_WINDOW_SIZE;
        } else {
            y += delta_y;
            height = desired_height;
        }
    }
    if edges.has_bottom() {
        height = (height + delta_y).max(MIN_WINDOW_SIZE);
    }

    SurfaceGeometry {
        x,
        y,
        width: width.max(MIN_WINDOW_SIZE) as u32,
        height: height.max(MIN_WINDOW_SIZE) as u32,
    }
}

#[cfg(test)]
mod tests {
    use nekoland_ecs::components::SurfaceGeometry;
    use nekoland_ecs::resources::{GlobalPointerPosition, ResizeEdges};

    use super::{WindowGrabMode, WindowGrabState, geometry_for_pointer};

    fn grab_state(mode: WindowGrabMode) -> WindowGrabState {
        WindowGrabState {
            surface_id: 7,
            mode,
            start_pointer_x: 100.0,
            start_pointer_y: 200.0,
            start_geometry: SurfaceGeometry { x: 40, y: 60, width: 800, height: 600 },
        }
    }

    #[test]
    fn move_grab_tracks_pointer_delta() {
        let geometry = geometry_for_pointer(
            &grab_state(WindowGrabMode::Move),
            &GlobalPointerPosition { x: 148.0, y: 236.0 },
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
        );

        assert_eq!(geometry.x, 776);
        assert_eq!(geometry.width, 64);
    }

    #[test]
    fn bottom_right_resize_expands_geometry_by_pointer_delta() {
        let geometry = geometry_for_pointer(
            &grab_state(WindowGrabMode::Resize { edges: ResizeEdges::BottomRight }),
            &GlobalPointerPosition { x: 164.0, y: 248.0 },
        );

        assert_eq!(geometry.width, 864);
        assert_eq!(geometry.height, 648);
    }
}
