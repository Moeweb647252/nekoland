use bevy_ecs::prelude::{Query, Res, With};
use nekoland_ecs::components::{LayoutSlot, SurfaceGeometry, WindowState, Workspace, XdgWindow};
use nekoland_ecs::resources::WorkArea;

/// Floating layout strategy.
///
/// Windows whose state is [`WindowState::Floating`] are positioned freely.
/// On first commit (geometry at the default origin `(0, 0)` with a real size),
/// the window is centred inside the current [`WorkArea`] so it appears in a
/// sensible location without the user having to drag it.  Subsequent frames
/// leave the geometry untouched, allowing free dragging and resizing.
///
/// Windows in any other state are ignored by this system; the fullscreen and
/// maximized geometries are applied by the `fullscreen_layout_system`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FloatingLayout;

pub fn floating_layout_system(
    mut windows: Query<(&mut SurfaceGeometry, &WindowState, &LayoutSlot), With<XdgWindow>>,
    workspaces: Query<&Workspace>,
    work_area: Res<WorkArea>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map(|workspace| workspace.id.0)
        .or_else(|| {
            workspaces.iter().min_by_key(|workspace| workspace.id).map(|workspace| workspace.id.0)
        });

    for (mut geometry, state, layout_slot) in &mut windows {
        if *state != WindowState::Floating {
            continue;
        }
        if active_workspace.is_some_and(|workspace| layout_slot.workspace != workspace) {
            continue;
        }

        // Centre newly created floating windows inside the work area.
        // "New" is detected by checking whether the window is still at the
        // default origin (0, 0) – the protocol layer spawns all windows there.
        if geometry.x == 0 && geometry.y == 0 && geometry.width > 0 && geometry.height > 0 {
            geometry.x = centre_x(&work_area, geometry.width);
            geometry.y = centre_y(&work_area, geometry.height);
        }
        // Existing windows with non-zero positions are left as-is, preserving
        // user-initiated moves.
    }

    tracing::trace!("floating layout system tick");
}

/// Horizontal centre-align within `work_area`.
pub(crate) fn centre_x(work_area: &WorkArea, width: u32) -> i32 {
    let available = work_area.width as i32;
    let window = width as i32;
    work_area.x + ((available - window) / 2).max(0)
}

/// Vertical centre-align within `work_area`.
pub(crate) fn centre_y(work_area: &WorkArea, height: u32) -> i32 {
    let available = work_area.height as i32;
    let window = height as i32;
    work_area.y + ((available - window) / 2).max(0)
}

#[cfg(test)]
mod tests {
    use nekoland_ecs::resources::WorkArea;

    use super::{centre_x, centre_y};

    fn work_area() -> WorkArea {
        WorkArea { x: 0, y: 32, width: 1280, height: 688 }
    }

    #[test]
    fn new_window_is_centred_horizontally() {
        // work_area: x=0 w=1280  →  (1280 - 800) / 2 = 240
        assert_eq!(centre_x(&work_area(), 800), 240);
    }

    #[test]
    fn new_window_is_centred_vertically() {
        // work_area: y=32 h=688  →  32 + (688 - 600) / 2 = 32 + 44 = 76
        assert_eq!(centre_y(&work_area(), 600), 76);
    }

    #[test]
    fn window_wider_than_work_area_is_placed_at_work_area_origin() {
        // clamp with max(0) prevents negative x
        assert_eq!(centre_x(&work_area(), 2000), 0);
    }

    #[test]
    fn window_taller_than_work_area_is_placed_at_work_area_top() {
        assert_eq!(centre_y(&work_area(), 900), 32);
    }
}
