use std::collections::BTreeSet;

use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, With};
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, LayoutSlot, ServerDecoration, SurfaceGeometry, WindowAnimation,
    WindowState, WlSurfaceHandle, Workspace, XdgWindow,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated};
use nekoland_ecs::resources::{
    CompositorConfig, PendingXdgRequests, WindowLifecycleAction, WindowLifecycleRequest,
    XdgSurfaceRole,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToplevelManager;

pub fn toplevel_lifecycle_system(
    mut commands: Commands,
    config: Res<CompositorConfig>,
    mut pending_xdg_requests: ResMut<PendingXdgRequests>,
    existing_surfaces: Query<&WlSurfaceHandle, With<XdgWindow>>,
    mut windows: Query<
        (
            Entity,
            &WlSurfaceHandle,
            &mut SurfaceGeometry,
            &mut BufferState,
            &mut XdgWindow,
            &WindowState,
        ),
        With<XdgWindow>,
    >,
    workspaces: Query<&Workspace>,
    mut window_created: MessageWriter<WindowCreated>,
    mut window_closed: MessageWriter<WindowClosed>,
) {
    let mut known_surfaces =
        existing_surfaces.iter().map(|surface| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in pending_xdg_requests.items.drain(..) {
        match request.action.clone() {
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Toplevel, size }
                if known_surfaces.insert(request.surface_id) =>
            {
                let workspace_id = workspaces
                    .iter()
                    .find(|workspace| workspace.active)
                    .map(|workspace| workspace.id.0)
                    .unwrap_or(1);
                let geometry = size
                    .unwrap_or(nekoland_ecs::resources::SurfaceExtent { width: 960, height: 720 });
                commands.spawn((
                    WindowBundle {
                        surface: WlSurfaceHandle { id: request.surface_id },
                        geometry: SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: geometry.width.max(1),
                            height: geometry.height.max(1),
                        },
                        buffer: BufferState { attached: true, scale: 1 },
                        window: XdgWindow {
                            app_id: "org.nekoland.demo".to_owned(),
                            title: format!("Window {}", request.surface_id),
                            last_acked_configure: None,
                        },
                        state: default_window_state(&config),
                        decoration: ServerDecoration { enabled: true },
                        border_theme: BorderTheme {
                            width: border_width(&config.default_layout),
                            color: config.border_color.clone(),
                        },
                        animation: WindowAnimation::default(),
                    },
                    LayoutSlot { workspace: workspace_id, column: 0, row: 0 },
                ));
                window_created.write(WindowCreated {
                    surface_id: request.surface_id,
                    title: format!("Window {}", request.surface_id),
                });
            }
            WindowLifecycleAction::Committed {
                role: XdgSurfaceRole::Toplevel,
                size: Some(size),
            } => {
                let mut handled = false;

                for (_, surface, mut geometry, mut buffer, _, state) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    buffer.attached = true;
                    if !matches!(
                        state,
                        WindowState::Maximized | WindowState::Fullscreen | WindowState::Hidden
                    ) {
                        geometry.width = size.width.max(1);
                        geometry.height = size.height.max(1);
                    }
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::Committed { role: XdgSurfaceRole::Toplevel, size: None } => {
                if !known_surfaces.contains(&request.surface_id) {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::ConfigureRequested { role: XdgSurfaceRole::Toplevel } => {
                tracing::trace!(surface_id = request.surface_id, "configure requested");
            }
            WindowLifecycleAction::Destroyed { role: XdgSurfaceRole::Toplevel } => {
                let mut handled = false;

                for (entity, surface, _, _, _, _) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    commands.entity(entity).despawn();
                    known_surfaces.remove(&request.surface_id);
                    window_closed.write(WindowClosed { surface_id: request.surface_id });
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            WindowLifecycleAction::MetadataChanged { title, app_id } => {
                let mut handled = false;

                for (_, surface, _, _, mut window, _) in &mut windows {
                    if surface.id != request.surface_id {
                        continue;
                    }

                    if let Some(title) = &title {
                        window.title = title.clone();
                    }
                    if let Some(app_id) = &app_id {
                        window.app_id = app_id.clone();
                    }
                    handled = true;
                    break;
                }

                if !handled {
                    deferred.push(request);
                }
            }
            _ => deferred.push(request),
        }
    }

    pending_xdg_requests.items = deferred;
}

#[allow(dead_code)]
fn _trace_unhandled_request(request: &WindowLifecycleRequest) {
    match &request.action {
        WindowLifecycleAction::Committed { role, size } => {
            tracing::trace!(surface_id = request.surface_id, ?role, ?size, "deferred xdg request");
        }
        WindowLifecycleAction::ConfigureRequested { role } => {
            tracing::trace!(surface_id = request.surface_id, ?role, "deferred xdg request");
        }
        WindowLifecycleAction::AckConfigure { role, serial } => {
            tracing::trace!(
                surface_id = request.surface_id,
                ?role,
                serial,
                "deferred xdg configure ack"
            );
        }
        WindowLifecycleAction::MetadataChanged { title, app_id } => {
            tracing::trace!(
                surface_id = request.surface_id,
                title = ?title,
                app_id = ?app_id,
                "deferred xdg metadata change"
            );
        }
        WindowLifecycleAction::InteractiveMove { seat_name, serial } => {
            tracing::trace!(
                surface_id = request.surface_id,
                seat_name,
                serial,
                "deferred interactive move request"
            );
        }
        WindowLifecycleAction::InteractiveResize { seat_name, serial, edges } => {
            tracing::trace!(
                surface_id = request.surface_id,
                seat_name,
                serial,
                edges,
                "deferred interactive resize request"
            );
        }
        WindowLifecycleAction::Maximize => {
            tracing::trace!(surface_id = request.surface_id, "deferred maximize request");
        }
        WindowLifecycleAction::UnMaximize => {
            tracing::trace!(surface_id = request.surface_id, "deferred unmaximize request");
        }
        WindowLifecycleAction::Fullscreen { output_name } => {
            tracing::trace!(
                surface_id = request.surface_id,
                output_name = ?output_name,
                "deferred fullscreen request"
            );
        }
        WindowLifecycleAction::UnFullscreen => {
            tracing::trace!(surface_id = request.surface_id, "deferred unfullscreen request");
        }
        WindowLifecycleAction::Minimize => {
            tracing::trace!(surface_id = request.surface_id, "deferred minimize request");
        }
        WindowLifecycleAction::PopupCreated { parent_surface_id, placement } => {
            tracing::trace!(
                surface_id = request.surface_id,
                parent_surface_id = ?parent_surface_id,
                placement = ?placement,
                "deferred popup create request"
            );
        }
        WindowLifecycleAction::PopupRepositioned { placement } => {
            tracing::trace!(
                surface_id = request.surface_id,
                placement = ?placement,
                "deferred popup reposition request"
            );
        }
        WindowLifecycleAction::PopupGrab { seat_name, serial } => {
            tracing::trace!(
                surface_id = request.surface_id,
                seat_name,
                serial,
                "deferred popup grab request"
            );
        }
        WindowLifecycleAction::Destroyed { role } => {
            tracing::trace!(surface_id = request.surface_id, ?role, "deferred destroy request");
        }
    }
}

fn default_window_state(config: &CompositorConfig) -> WindowState {
    match config.default_layout.as_str() {
        "floating" => WindowState::Floating,
        "maximized" => WindowState::Maximized,
        "fullscreen" => WindowState::Fullscreen,
        "tiling" | "stacking" => WindowState::Floating,
        _ => WindowState::Floating,
    }
}

fn border_width(default_layout: &str) -> u32 {
    if matches!(default_layout, "tiling" | "stacking") { 2 } else { 1 }
}
