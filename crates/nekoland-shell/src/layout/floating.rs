use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Local, Query, Res, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{
    OutputProperties, OutputViewport, SurfaceGeometry, WindowLayout, WindowMode, WindowPlacement,
    WindowPosition, WindowSceneGeometry, XdgWindow,
};
use nekoland_ecs::resources::{WaylandIngress, WorkArea};
use nekoland_ecs::views::WorkspaceRuntime;
use nekoland_ecs::views::{OutputRuntime, WindowRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::{
    initialize_scene_geometry_from_surface, preferred_primary_output_id, project_scene_geometry,
    resolve_output_state_for_workspace,
};

const MIN_FLOATING_WINDOW_SIZE: u32 = 32;

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
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    wayland_ingress: Res<WaylandIngress>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
    work_area: Res<WorkArea>,
) {
    let primary_output_id = preferred_primary_output_id(Some(&wayland_ingress));
    let placement_area = placement_work_area(
        &work_area,
        resolve_output_state_for_workspace(&outputs, None, primary_output_id)
            .map(|(_, output, _, _)| output),
    );
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
        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces);
        let output_state =
            resolve_output_state_for_workspace(&outputs, workspace_id, primary_output_id);
        let window_work_area =
            output_state.as_ref().map_or(placement_area, |(_, output, _, work_area)| {
                placement_work_area(
                    &WorkArea {
                        x: work_area.x,
                        y: work_area.y,
                        width: work_area.width,
                        height: work_area.height,
                    },
                    Some(output),
                )
            });
        let fallback_viewport = OutputViewport::default();
        let viewport = output_state
            .as_ref()
            .map(|(_, _, viewport, _)| &**viewport)
            .unwrap_or(&fallback_viewport);
        initialize_scene_geometry_from_surface(
            &mut window.scene_geometry,
            &window.geometry,
            viewport,
        );

        if let Some(size) = window.placement.floating_size {
            window.scene_geometry.width = size.width.max(MIN_FLOATING_WINDOW_SIZE);
            window.scene_geometry.height = size.height.max(MIN_FLOATING_WINDOW_SIZE);
        }

        if window.management_hints.client_driven_resize && has_explicit_placement {
            window.scene_geometry.width = window.scene_geometry.width.max(MIN_FLOATING_WINDOW_SIZE);
            window.scene_geometry.height =
                window.scene_geometry.height.max(MIN_FLOATING_WINDOW_SIZE);
        } else {
            let max_width = window_work_area.width.max(MIN_FLOATING_WINDOW_SIZE);
            let max_height = window_work_area.height.max(MIN_FLOATING_WINDOW_SIZE);
            window.scene_geometry.width =
                window.scene_geometry.width.clamp(MIN_FLOATING_WINDOW_SIZE, max_width);
            window.scene_geometry.height =
                window.scene_geometry.height.clamp(MIN_FLOATING_WINDOW_SIZE, max_height);
        }

        if should_reposition_floating_window(
            &window.placement,
            &window.scene_geometry,
            work_area_changed,
        ) {
            window.placement.set_auto_position(WindowPosition {
                x: centre_x(&window_work_area, viewport.origin_x, window.scene_geometry.width),
                y: centre_y(&window_work_area, viewport.origin_y, window.scene_geometry.height),
            });
        }

        if let Some(position) = window.placement.resolved_floating_position() {
            window.scene_geometry.x = position.x;
            window.scene_geometry.y = position.y;
        }

        if output_state.is_some() {
            *window.geometry = project_scene_geometry(&window.scene_geometry, viewport);
        } else {
            *window.geometry = SurfaceGeometry {
                x: window.scene_geometry.x.clamp(i32::MIN as isize, i32::MAX as isize) as i32,
                y: window.scene_geometry.y.clamp(i32::MIN as isize, i32::MAX as isize) as i32,
                width: window.scene_geometry.width,
                height: window.scene_geometry.height,
            };
        }
    }

    *previous_work_area = Some(placement_area);
    tracing::trace!("floating layout system tick");
}

/// Identifies windows that still look "unplaced" and should be auto-centered when they first
/// become visible.
pub(crate) fn should_auto_place_floating_window(
    placement: &WindowPlacement,
    geometry: &WindowSceneGeometry,
) -> bool {
    placement.should_auto_place(geometry)
}

/// Re-centers unpositioned floating windows, and also lets unpositioned windows follow work-area
/// changes until the user explicitly places them.
fn should_reposition_floating_window(
    placement: &WindowPlacement,
    geometry: &WindowSceneGeometry,
    work_area_changed: bool,
) -> bool {
    placement.should_reposition_auto(geometry, work_area_changed)
}

/// Horizontal centre-align within `work_area`.
pub(crate) fn centre_x(work_area: &WorkArea, viewport_origin_x: isize, width: u32) -> isize {
    let available = work_area.width as isize;
    let window = width as isize;
    viewport_origin_x
        .saturating_add(work_area.x as isize)
        .saturating_add(((available - window) / 2).max(0))
}

/// Vertical centre-align within `work_area`.
pub(crate) fn centre_y(work_area: &WorkArea, viewport_origin_y: isize, height: u32) -> isize {
    let available = work_area.height as isize;
    let window = height as isize;
    viewport_origin_y
        .saturating_add(work_area.y as isize)
        .saturating_add(((available - window) / 2).max(0))
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
        BufferState, FloatingPosition, SurfaceGeometry, WindowLayout, WindowManagementHints,
        WindowMode, WindowPlacement, WindowPosition, WindowSize, WlSurfaceHandle,
    };
    use nekoland_ecs::resources::{WaylandIngress, WorkArea};

    use super::{centre_x, centre_y, floating_layout_system};

    fn work_area() -> WorkArea {
        WorkArea { x: 0, y: 32, width: 1280, height: 688 }
    }

    #[test]
    fn new_window_is_centred_horizontally() {
        // work_area: x=0 w=1280  →  (1280 - 800) / 2 = 240
        assert_eq!(centre_x(&work_area(), 0, 800), 240);
    }

    #[test]
    fn new_window_is_centred_vertically() {
        // work_area: y=32 h=688  →  32 + (688 - 600) / 2 = 32 + 44 = 76
        assert_eq!(centre_y(&work_area(), 0, 600), 76);
    }

    #[test]
    fn window_wider_than_work_area_is_placed_at_work_area_origin() {
        // clamp with max(0) prevents negative x
        assert_eq!(centre_x(&work_area(), 0, 2000), 0);
    }

    #[test]
    fn compositor_managed_floating_layout_clamps_window_size_to_work_area() {
        let mut app = NekolandApp::new("floating-layout-compositor-clamp-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .inner_mut()
            .add_systems(LayoutSchedule, floating_layout_system);

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 99 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 3000, height: 2000 },
                buffer: BufferState { attached: true, scale: 1 },
                management_hints: WindowManagementHints::x11(false, false, false, None),
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(window) else {
            panic!("window geometry");
        };
        assert_eq!(geometry.width, 1280);
        assert_eq!(geometry.height, 720);
    }

    #[test]
    fn native_wayland_initial_floating_layout_clamps_unplaced_size_to_work_area() {
        let mut app = NekolandApp::new("floating-layout-native-wayland-initial-fit-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .inner_mut()
            .add_systems(LayoutSchedule, floating_layout_system);

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 100 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 3000, height: 2000 },
                buffer: BufferState { attached: true, scale: 1 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(window) else {
            panic!("window geometry");
        };
        assert_eq!(geometry.width, 1280);
        assert_eq!(geometry.height, 720);
    }

    #[test]
    fn native_wayland_explicitly_sized_windows_can_exceed_work_area() {
        let mut app = NekolandApp::new("floating-layout-native-wayland-explicit-size-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1280, height: 720 })
            .inner_mut()
            .add_systems(LayoutSchedule, floating_layout_system);

        let window = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 101 },
                    geometry: SurfaceGeometry { x: 0, y: 0, width: 3000, height: 2000 },
                    buffer: BufferState { attached: true, scale: 1 },
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    ..Default::default()
                },
                WindowPlacement {
                    floating_position: Some(FloatingPosition::Explicit(WindowPosition {
                        x: 0,
                        y: 0,
                    })),
                    floating_size: Some(WindowSize { width: 3000, height: 2000 }),
                },
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(window) else {
            panic!("window geometry");
        };
        assert_eq!(geometry.width, 3000);
        assert_eq!(geometry.height, 2000);
    }

    #[test]
    fn window_taller_than_work_area_is_placed_at_work_area_top() {
        assert_eq!(centre_y(&work_area(), 0, 900), 32);
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
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1024, height: 768 })
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

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(entity) else {
            panic!("floating layout should keep geometry after work area updates");
        };
        assert_eq!((geometry.x, geometry.y), (944, 524));
    }

    #[test]
    fn explicit_placement_applies_even_before_buffer_attach() {
        let mut app = NekolandApp::new("floating-layout-explicit-placement-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 1024, height: 768 })
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

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(entity) else {
            panic!("floating layout should apply explicit placement before first attach");
        };
        assert_eq!((geometry.x, geometry.y, geometry.width, geometry.height), (900, 120, 777, 555));
    }

    fn run_floating_layout(
        geometry: SurfaceGeometry,
        placement: WindowPlacement,
    ) -> SurfaceGeometry {
        let mut app = NekolandApp::new("floating-layout-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(work_area())
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
        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(entity) else {
            panic!("floating layout should keep the window geometry component");
        };
        geometry.clone()
    }
}
