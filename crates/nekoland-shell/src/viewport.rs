use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Res, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{
    OutputProperties, OutputViewport, OutputWorkArea, SurfaceGeometry, WindowMode,
    WindowSceneGeometry, XdgWindow,
};
use nekoland_ecs::resources::PrimaryOutputState;
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

pub(crate) fn initialize_scene_geometry_from_surface(
    scene_geometry: &mut WindowSceneGeometry,
    geometry: &SurfaceGeometry,
    viewport: &OutputViewport,
) {
    if scene_geometry.width != 0 || scene_geometry.height != 0 {
        return;
    }

    scene_geometry.x = viewport.origin_x.saturating_add(geometry.x as isize);
    scene_geometry.y = viewport.origin_y.saturating_add(geometry.y as isize);
    scene_geometry.width = geometry.width;
    scene_geometry.height = geometry.height;
}

pub(crate) fn scene_geometry_intersects_viewport(
    scene_geometry: &WindowSceneGeometry,
    viewport: &OutputViewport,
    output: &OutputProperties,
) -> bool {
    if scene_geometry.width == 0 || scene_geometry.height == 0 {
        return false;
    }

    let left = scene_geometry.x as i128;
    let top = scene_geometry.y as i128;
    let right = left.saturating_add(i128::from(scene_geometry.width));
    let bottom = top.saturating_add(i128::from(scene_geometry.height));

    let viewport_left = viewport.origin_x as i128;
    let viewport_top = viewport.origin_y as i128;
    let viewport_right = viewport_left.saturating_add(i128::from(output.width.max(1)));
    let viewport_bottom = viewport_top.saturating_add(i128::from(output.height.max(1)));

    left < viewport_right && right > viewport_left && top < viewport_bottom && bottom > viewport_top
}

pub(crate) fn project_scene_geometry(
    scene_geometry: &WindowSceneGeometry,
    viewport: &OutputViewport,
) -> SurfaceGeometry {
    SurfaceGeometry {
        x: saturating_isize_to_i32(scene_geometry.x.saturating_sub(viewport.origin_x)),
        y: saturating_isize_to_i32(scene_geometry.y.saturating_sub(viewport.origin_y)),
        width: scene_geometry.width,
        height: scene_geometry.height,
    }
}

#[allow(dead_code)]
pub(crate) fn center_viewport_on_scene_geometry(
    viewport: &mut OutputViewport,
    scene_geometry: &WindowSceneGeometry,
    output: &OutputProperties,
) {
    let half_width = (output.width / 2) as isize;
    let half_height = (output.height / 2) as isize;
    let target_x = scene_geometry.x.saturating_add((scene_geometry.width / 2) as isize);
    let target_y = scene_geometry.y.saturating_add((scene_geometry.height / 2) as isize);
    viewport.origin_x = target_x.saturating_sub(half_width);
    viewport.origin_y = target_y.saturating_sub(half_height);
}

#[allow(dead_code)]
pub(crate) fn resolve_active_output_state<'w, 's>(
    outputs: &'w Query<'w, 's, OutputRuntime>,
    primary_output: Option<&PrimaryOutputState>,
) -> Option<(&'w OutputProperties, &'w OutputViewport)> {
    if let Some(primary_output_name) = primary_output.and_then(|primary| primary.name.as_deref())
        && let Some(output) = outputs.iter().find(|output| output.name() == primary_output_name)
    {
        return Some((output.properties, output.viewport));
    }

    outputs.iter().next().map(|output| (output.properties, output.viewport))
}

pub(crate) fn resolve_output_state_for_workspace<'w, 's>(
    outputs: &'w Query<'w, 's, (Entity, OutputRuntime)>,
    workspace_id: Option<u32>,
    primary_output: Option<&PrimaryOutputState>,
) -> Option<(String, &'w OutputProperties, &'w OutputViewport, &'w OutputWorkArea)> {
    if let Some(workspace_id) = workspace_id
        && let Some((_, output)) = outputs.iter().find(|(_, output)| {
            output
                .current_workspace
                .as_ref()
                .is_some_and(|current_workspace| current_workspace.workspace.0 == workspace_id)
        })
    {
        return Some((
            output.name().to_owned(),
            output.properties,
            output.viewport,
            output.work_area,
        ));
    }

    if let Some(primary_output_name) = primary_output.and_then(|primary| primary.name.as_deref())
        && let Some((_, output)) =
            outputs.iter().find(|(_, output)| output.name() == primary_output_name)
    {
        return Some((
            output.name().to_owned(),
            output.properties,
            output.viewport,
            output.work_area,
        ));
    }

    outputs.iter().next().map(|(_, output)| {
        (output.name().to_owned(), output.properties, output.viewport, output.work_area)
    })
}

pub(crate) fn resolve_output_state_by_name<'w, 's>(
    outputs: &'w Query<'w, 's, (Entity, OutputRuntime)>,
    output_name: &str,
) -> Option<(&'w OutputProperties, &'w OutputViewport, &'w OutputWorkArea)> {
    outputs
        .iter()
        .find(|(_, output)| output.name() == output_name)
        .map(|(_, output)| (output.properties, output.viewport, output.work_area))
}

pub fn window_viewport_projection_system(
    outputs: Query<(Entity, OutputRuntime)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    workspaces: Query<(Entity, WorkspaceRuntime)>,
    mut windows: Query<WindowRuntime, (With<XdgWindow>, Allow<Disabled>)>,
) {
    for mut window in &mut windows {
        if let Some(background) = window.background.as_ref() {
            let Some((output, _, _)) = resolve_output_state_by_name(&outputs, &background.output)
            else {
                *window.viewport_visibility =
                    nekoland_ecs::components::WindowViewportVisibility::default();
                continue;
            };
            window.viewport_visibility.visible = *window.mode != WindowMode::Hidden;
            window.viewport_visibility.output = Some(background.output.clone());
            window.geometry.x = 0;
            window.geometry.y = 0;
            window.geometry.width = output.width.max(1);
            window.geometry.height = output.height.max(1);
            continue;
        }

        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces);
        let Some((output_name, output, viewport, _)) =
            resolve_output_state_for_workspace(&outputs, workspace_id, primary_output.as_deref())
        else {
            *window.viewport_visibility =
                nekoland_ecs::components::WindowViewportVisibility { visible: false, output: None };
            continue;
        };

        let visible = match *window.mode {
            WindowMode::Hidden => false,
            WindowMode::Maximized | WindowMode::Fullscreen => true,
            WindowMode::Normal => {
                initialize_scene_geometry_from_surface(
                    &mut window.scene_geometry,
                    &window.geometry,
                    viewport,
                );
                let projected_visible =
                    scene_geometry_intersects_viewport(&window.scene_geometry, viewport, output);
                *window.geometry = project_scene_geometry(&window.scene_geometry, viewport);
                projected_visible
            }
        };

        window.viewport_visibility.visible = visible;
        window.viewport_visibility.output = visible.then_some(output_name);
    }

    tracing::trace!("window viewport projection tick");
}

fn saturating_isize_to_i32(value: isize) -> i32 {
    value.clamp(i32::MIN as isize, i32::MAX as isize) as i32
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::With;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        OutputDevice, OutputKind, OutputProperties, SurfaceGeometry, WindowLayout, WindowMode,
        WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
    };

    use super::{OutputViewport, window_viewport_projection_system};

    #[test]
    fn viewport_projection_moves_visible_window_into_output_local_coordinates() {
        let mut app = NekolandApp::new("viewport-projection-test");
        app.inner_mut().add_systems(LayoutSchedule, window_viewport_projection_system);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: "test".to_owned(),
            },
            properties: OutputProperties {
                width: 800,
                height: 600,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let Ok(output) = app
            .inner_mut()
            .world_mut()
            .query_filtered::<bevy_ecs::prelude::Entity, With<OutputDevice>>()
            .single(app.inner_mut().world_mut())
        else {
            panic!("output entity");
        };
        app.inner_mut()
            .world_mut()
            .entity_mut(output)
            .insert(OutputViewport { origin_x: 1000, origin_y: 2000 });

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 11 },
                geometry: SurfaceGeometry { x: 0, y: 0, width: 320, height: 240 },
                window: XdgWindow::default(),
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            })
            .id();
        app.inner_mut().world_mut().entity_mut(window).insert(WindowSceneGeometry {
            x: 1120,
            y: 2090,
            width: 320,
            height: 240,
        });

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let Some(geometry) = app.inner().world().get::<SurfaceGeometry>(window) else {
            panic!("window geometry");
        };
        assert_eq!((geometry.x, geometry.y), (120, 90));
    }
}
