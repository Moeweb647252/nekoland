use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Query, Res, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{WindowMode, XdgWindow};
use nekoland_ecs::resources::{WaylandIngress, WorkArea};
use nekoland_ecs::views::{OutputRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::window_workspace_runtime_id;

use crate::viewport::{
    preferred_primary_output_id, resolve_output_state_for_window,
    resolve_output_state_for_workspace,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullscreenLayout;

/// Applies fullscreen and maximized geometry after layout/work-area state has been updated.
pub fn fullscreen_layout_system(
    outputs: Query<(bevy_ecs::prelude::Entity, OutputRuntime)>,
    mut windows: Query<WindowRuntime, With<XdgWindow>>,
    workspaces: Query<(bevy_ecs::prelude::Entity, WorkspaceRuntime), Allow<Disabled>>,
    wayland_ingress: Res<WaylandIngress>,
    work_area: Res<WorkArea>,
) {
    let primary_output_id = preferred_primary_output_id(Some(&wayland_ingress));
    for mut window in &mut windows {
        if !window.role.is_managed() {
            continue;
        }
        let workspace_id = window_workspace_runtime_id(window.child_of, &workspaces);
        match *window.mode {
            WindowMode::Fullscreen => {
                let Some((_, output, _, _)) = resolve_output_state_for_window(
                    &outputs,
                    workspace_id,
                    Some(window.fullscreen_target.as_ref()),
                    primary_output_id,
                ) else {
                    continue;
                };
                window.geometry.x = 0;
                window.geometry.y = 0;
                window.geometry.width = output.width.max(1);
                window.geometry.height = output.height.max(1);
            }
            WindowMode::Maximized => {
                let output_state =
                    resolve_output_state_for_workspace(&outputs, workspace_id, primary_output_id);
                let window_work_area =
                    output_state.as_ref().map_or(*work_area, |(_, _, _, work_area)| WorkArea {
                        x: work_area.x,
                        y: work_area.y,
                        width: work_area.width,
                        height: work_area.height,
                    });
                // Keep a small inset so maximized windows still leave room for compositor-side
                // borders and do not visually merge into the output edge.
                window.geometry.x = window_work_area.x + 16;
                window.geometry.y = window_work_area.y + 16;
                window.geometry.width = window_work_area.width.saturating_sub(32).max(1);
                window.geometry.height = window_work_area.height.saturating_sub(32).max(1);
            }
            _ => {}
        }
    }

    tracing::trace!("fullscreen layout system tick");
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        OutputCurrentWorkspace, OutputDevice, OutputKind, OutputProperties, OutputViewport,
        OutputWorkArea, SurfaceGeometry, WindowFullscreenTarget, WindowLayout, WindowMode,
        WindowSceneGeometry, WlSurfaceHandle, Workspace, WorkspaceId, XdgWindow,
    };
    use nekoland_ecs::resources::{WaylandIngress, WorkArea};
    use nekoland_ecs::selectors::OutputName;

    use crate::viewport::window_viewport_projection_system;

    use super::fullscreen_layout_system;

    #[test]
    fn fullscreen_layout_prefers_named_target_output_over_workspace_output() {
        let mut app = NekolandApp::new("fullscreen-target-output-test");
        app.insert_resource(WaylandIngress::default())
            .insert_resource(WorkArea { x: 0, y: 0, width: 800, height: 600 });
        app.inner_mut().add_systems(
            LayoutSchedule,
            (fullscreen_layout_system, window_viewport_projection_system).chain(),
        );

        let workspace_1 = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(2),
            name: "2".to_owned(),
            active: false,
        });

        let virtual_output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "Virtual-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "one".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 800,
                        height: 600,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    viewport: OutputViewport::default(),
                    work_area: OutputWorkArea { x: 0, y: 0, width: 800, height: 600 },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(1) },
            ))
            .id();
        let hdmi_output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "HDMI-A-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "test".to_owned(),
                        model: "two".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1920,
                        height: 1080,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                    viewport: OutputViewport::default(),
                    work_area: OutputWorkArea { x: 0, y: 0, width: 1920, height: 1080 },
                    ..Default::default()
                },
                OutputCurrentWorkspace { workspace: WorkspaceId(2) },
            ))
            .id();
        let virtual_output_id = *app
            .inner()
            .world()
            .get::<nekoland_ecs::components::OutputId>(virtual_output)
            .expect("virtual output id");
        let hdmi_output_id = *app
            .inner()
            .world()
            .get::<nekoland_ecs::components::OutputId>(hdmi_output)
            .expect("hdmi output id");
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(virtual_output_id);

        let window_entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 77 },
                    geometry: SurfaceGeometry { x: 30, y: 40, width: 400, height: 300 },
                    scene_geometry: WindowSceneGeometry { x: 30, y: 40, width: 400, height: 300 },
                    viewport_visibility: Default::default(),
                    buffer: Default::default(),
                    content_version: Default::default(),
                    window: XdgWindow::default(),
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Fullscreen,
                    decoration: Default::default(),
                    border_theme: Default::default(),
                    animation: Default::default(),
                },
                WindowFullscreenTarget { output: Some(OutputName::from("HDMI-A-1")) },
                ChildOf(workspace_1),
            ))
            .id();

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner().world();
        let Some(geometry) = world.get::<SurfaceGeometry>(window_entity) else {
            panic!("fullscreen window geometry should exist");
        };
        let Some(visibility) =
            world.get::<nekoland_ecs::components::WindowViewportVisibility>(window_entity)
        else {
            panic!("fullscreen window viewport visibility should exist");
        };

        assert_eq!((geometry.x, geometry.y), (0, 0));
        assert_eq!((geometry.width, geometry.height), (1920, 1080));
        assert!(visibility.visible);
        assert_eq!(visibility.output, Some(hdmi_output_id));
    }
}
