use bevy_ecs::prelude::{Local, Query, Res, ResMut, With};
use nekoland_ecs::components::{
    LayoutSlot, SurfaceGeometry, WindowState, WlSurfaceHandle, Workspace, X11Window, XdgWindow,
};
use nekoland_ecs::resources::{
    CompositorConfig, GlobalPointerPosition, KeyboardFocusState, PendingWindowServerRequests,
    WindowServerAction,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusManager;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusHoverState {
    initialized: bool,
    hovered_surface: Option<u64>,
}

pub fn window_focus_request_system(
    mut pending_window_requests: ResMut<PendingWindowServerRequests>,
    windows: Query<
        (&WlSurfaceHandle, &WindowState, &LayoutSlot, Option<&X11Window>),
        With<XdgWindow>,
    >,
    workspaces: Query<&Workspace>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
) {
    let mut deferred = Vec::new();
    let active_workspace = active_workspace_id(&workspaces);

    for request in pending_window_requests.items.drain(..) {
        match request.action {
            WindowServerAction::Focus => {
                let Some((surface, state, layout_slot, x11_window)) =
                    windows.iter().find(|(surface, ..)| surface.id == request.surface_id)
                else {
                    deferred.push(request);
                    continue;
                };

                if *state != WindowState::Hidden
                    && x11_window.is_none_or(|window| !window.override_redirect)
                    && active_workspace.is_none_or(|workspace| layout_slot.workspace == workspace)
                {
                    keyboard_focus.focused_surface = Some(surface.id);
                } else {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_window_requests.items = deferred;
}

pub fn focus_management_system(
    config: Res<CompositorConfig>,
    pointer: Res<GlobalPointerPosition>,
    mut keyboard_focus: ResMut<KeyboardFocusState>,
    mut hover_state: Local<FocusHoverState>,
    windows: Query<
        (&WlSurfaceHandle, &SurfaceGeometry, &WindowState, &LayoutSlot, Option<&X11Window>),
        With<XdgWindow>,
    >,
    workspaces: Query<&Workspace>,
) {
    let active_workspace = active_workspace_id(&workspaces);
    let visible_windows = windows
        .iter()
        .filter_map(|(surface, geometry, state, layout_slot, x11_window)| {
            (*state != WindowState::Hidden
                && x11_window.is_none_or(|window| !window.override_redirect)
                && active_workspace.is_none_or(|workspace| layout_slot.workspace == workspace))
            .then_some((surface.id, geometry.clone()))
        })
        .collect::<Vec<_>>();
    let visible_surfaces =
        visible_windows.iter().map(|(surface_id, _)| *surface_id).collect::<Vec<_>>();

    if config.focus_follows_mouse {
        let hovered_surface = visible_windows.iter().rev().find_map(|(surface_id, geometry)| {
            pointer_in_geometry(pointer.x, pointer.y, geometry).then_some(*surface_id)
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
        keyboard_focus.focused_surface = visible_surfaces.first().copied();
    }

    tracing::trace!(focused_surface = ?keyboard_focus.focused_surface, "focus management tick");
}

fn pointer_in_geometry(pointer_x: f64, pointer_y: f64, geometry: &SurfaceGeometry) -> bool {
    let left = f64::from(geometry.x);
    let top = f64::from(geometry.y);
    let right = left + f64::from(geometry.width);
    let bottom = top + f64::from(geometry.height);

    pointer_x >= left && pointer_x < right && pointer_y >= top && pointer_y < bottom
}

fn active_workspace_id(workspaces: &Query<&Workspace>) -> Option<u32> {
    workspaces.iter().find(|workspace| workspace.active).map(|workspace| workspace.id.0).or_else(
        || workspaces.iter().min_by_key(|workspace| workspace.id).map(|workspace| workspace.id.0),
    )
}
