use bevy_ecs::prelude::{Local, Query, Res, With};
use nekoland_ecs::components::{
    OutputProperties, SurfaceGeometry, WindowLayout, WindowMode, WindowPlacement, WindowPosition,
    XdgWindow,
};
use nekoland_ecs::resources::WorkArea;
use nekoland_ecs::views::WindowRuntime;

/// Floating layout strategy.
///
/// Windows whose layout resolves to floating semantics keep their geometry but receive initial
/// placement and explicit placement-hint reconciliation here.
/// On first commit (geometry at the default origin `(0, 0)` with a real size), the window is
/// centred inside the current [`WorkArea`] so it appears in a sensible location without the user
/// having to drag it. Subsequent frames leave the geometry untouched unless placement hints change.
///
/// Fullscreen and maximized geometry constraints are applied later by
/// `fullscreen_layout_system`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FloatingLayout;

pub fn floating_layout_system(
    mut previous_work_area: Local<Option<WorkArea>>,
    mut windows: Query<WindowRuntime, With<XdgWindow>>,
    outputs: Query<&OutputProperties>,
    work_area: Res<WorkArea>,
) {
    let placement_area = placement_work_area(&work_area, outputs.iter().next());
    let work_area_changed =
        previous_work_area.as_ref().is_some_and(|previous| *previous != placement_area);

    for mut window in &mut windows {
        if !matches!(*window.layout, WindowLayout::Floating) {
            continue;
        }
        if matches!(
            *window.mode,
            WindowMode::Maximized | WindowMode::Fullscreen | WindowMode::Hidden
        ) {
            continue;
        }
        let has_explicit_placement = window.has_explicit_placement();
        let Some(buffer) = window.buffer.as_ref() else {
            continue;
        };
        if !buffer.attached && work_area.x == 0 && work_area.y == 0 && !has_explicit_placement {
            continue;
        }
        if window.geometry.width == 0 || window.geometry.height == 0 {
            continue;
        }

        if let Some(size) = window.placement.floating_size {
            window.geometry.width = size.width.max(64);
            window.geometry.height = size.height.max(64);
        }

        if should_reposition_floating_window(&window.placement, &window.geometry, work_area_changed)
        {
            window.placement.set_auto_position(WindowPosition {
                x: centre_x(&placement_area, window.geometry.width),
                y: centre_y(&placement_area, window.geometry.height),
            });
        }

        if let Some(position) = window.placement.resolved_floating_position() {
            window.geometry.x = position.x;
            window.geometry.y = position.y;
        }
    }

    *previous_work_area = Some(placement_area);
    tracing::trace!("floating layout system tick");
}

/// Identifies windows that still look "unplaced" and should be auto-centered when they first
/// become visible.
pub(crate) fn should_auto_place_floating_window(
    placement: &WindowPlacement,
    geometry: &SurfaceGeometry,
) -> bool {
    placement.should_auto_place(geometry)
}

/// Re-centers unpositioned floating windows, and also lets unpositioned windows follow work-area
/// changes until the user explicitly places them.
fn should_reposition_floating_window(
    placement: &WindowPlacement,
    geometry: &SurfaceGeometry,
    work_area_changed: bool,
) -> bool {
    placement.should_reposition_auto(geometry, work_area_changed)
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

/// Uses the full output rect as a temporary placement area while the real work area is still at
/// its startup fallback value.
pub(crate) fn placement_work_area(
    work_area: &WorkArea,
    output: Option<&OutputProperties>,
) -> WorkArea {
    let Some(output) = output else {
        return *work_area;
    };

    if work_area.x == 0
        && work_area.y == 0
        && (work_area.width < output.width || work_area.height < output.height)
    {
        WorkArea { x: 0, y: 0, width: output.width.max(1), height: output.height.max(1) }
    } else {
        *work_area
    }
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        BufferState, FloatingPosition, SurfaceGeometry, WindowLayout, WindowMode, WindowPlacement,
        WindowPosition, WindowSize, WlSurfaceHandle,
    };
    use nekoland_ecs::resources::WorkArea;

    use super::{centre_x, centre_y, floating_layout_system};

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

    #[test]
    fn floating_layout_preserves_existing_non_origin_geometry() {
        let geometry = run_floating_layout(
            SurfaceGeometry { x: 400, y: 48, width: 320, height: 240 },
            WindowPlacement::default(),
        );

        assert_eq!(geometry, SurfaceGeometry { x: 400, y: 48, width: 320, height: 240 });
    }

    #[test]
    fn floating_layout_centers_unpositioned_origin_windows() {
        let geometry = run_floating_layout(
            SurfaceGeometry { x: 0, y: 0, width: 320, height: 240 },
            WindowPlacement::default(),
        );

        assert_eq!(geometry.x, 480);
        assert_eq!(geometry.y, 256);
        assert_eq!(geometry.width, 320);
        assert_eq!(geometry.height, 240);
    }

    #[test]
    fn floating_layout_recenters_unpositioned_windows_when_work_area_changes() {
        let mut app = NekolandApp::new("floating-layout-work-area-test");
        app.insert_resource(WorkArea { x: 0, y: 0, width: 1024, height: 768 })
            .inner_mut()
            .add_systems(LayoutSchedule, floating_layout_system);

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 32, height: 32 },
                buffer: BufferState { attached: true, scale: 1 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            },))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().insert_resource(WorkArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        });
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let geometry = app
            .inner()
            .world()
            .get::<SurfaceGeometry>(entity)
            .expect("floating layout should keep geometry after work area updates");
        assert_eq!((geometry.x, geometry.y), (944, 524));
    }

    #[test]
    fn explicit_placement_applies_even_before_buffer_attach() {
        let mut app = NekolandApp::new("floating-layout-explicit-placement-test");
        app.insert_resource(WorkArea { x: 0, y: 0, width: 1024, height: 768 })
            .inner_mut()
            .add_systems(LayoutSchedule, floating_layout_system);

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((WindowBundle {
                surface: WlSurfaceHandle { id: 15 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 320, height: 240 },
                buffer: BufferState { attached: false, scale: 1 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            },))
            .id();
        app.inner_mut().world_mut().entity_mut(entity).insert(WindowPlacement {
            floating_position: Some(FloatingPosition::Explicit(WindowPosition { x: 900, y: 120 })),
            floating_size: Some(WindowSize { width: 777, height: 555 }),
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let geometry = app
            .inner()
            .world()
            .get::<SurfaceGeometry>(entity)
            .expect("floating layout should apply explicit placement before first attach");
        assert_eq!((geometry.x, geometry.y, geometry.width, geometry.height), (900, 120, 777, 555));
    }

    fn run_floating_layout(
        geometry: SurfaceGeometry,
        placement: WindowPlacement,
    ) -> SurfaceGeometry {
        let mut app = NekolandApp::new("floating-layout-test");
        app.insert_resource(work_area())
            .inner_mut()
            .add_systems(LayoutSchedule, floating_layout_system);

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 7 },
                    geometry,
                    buffer: BufferState { attached: true, scale: 1 },
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                placement,
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner()
            .world()
            .get::<SurfaceGeometry>(entity)
            .expect("floating layout should keep the window geometry component")
            .clone()
    }
}
