use std::collections::BTreeSet;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, Resource, With, Without};
use bevy_ecs::query::Allow;
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::{CompositorConfig, WindowRuleContext};
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    BorderTheme, BufferState, PendingInteractiveResize, ServerDecoration, SurfaceContentVersion,
    PopupSurface, SurfaceGeometry, Window, WindowAnimation, WindowFullscreenTarget, WindowLayout,
    WindowManagementHints, WindowMode, WindowPlacement, WindowPolicyState, WindowPosition,
    WindowRole, WindowSceneGeometry, WindowSize, WlSurfaceHandle,
};
use nekoland_ecs::events::{WindowClosed, WindowCreated, WindowMoved};
use nekoland_ecs::resources::{
    EntityIndex, FocusedOutputState, GlobalPointerPosition, KeyboardFocusState,
    PendingPopupServerRequests, PendingWindowEvents, PopupServerAction, PopupServerRequest,
    ResizeEdges, SurfaceExtent, WaylandIngress, WindowEvent, WindowEventRequest,
    WindowManagerRequest, WorkArea,
};
use nekoland_ecs::views::{OutputRuntime, PopupRuntime, WindowRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::focused_or_primary_workspace_runtime_target;

use crate::interaction::{ActiveWindowGrab, WindowGrabMode, begin_window_grab};
use crate::layout::floating::{centre_x, centre_y, placement_work_area, should_auto_place_floating_window};
use crate::viewport::{preferred_primary_output_id, resolve_output_state_for_workspace};
use crate::window_policy::{
    WindowBackgroundState, enter_temporary_window_mode,
    refresh_window_policy, resolve_background_output_id, restore_window_state,
    sync_window_background_role,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Resource)]
pub(crate) struct DeferredWindowEvents(PendingWindowEvents);

impl DeferredWindowEvents {
    pub(crate) fn take(&mut self) -> Vec<WindowEventRequest> {
        self.0.take()
    }

    pub(crate) fn replace(&mut self, requests: Vec<WindowEventRequest>) {
        self.0.replace(requests);
    }
}

type LifecycleWindows<'w, 's> =
    Query<'w, 's, (Entity, WindowRuntime), (With<Window>, Allow<Disabled>)>;
type LifecyclePopups<'w, 's> =
    Query<'w, 's, PopupRuntime, (With<PopupSurface>, Without<Window>, Allow<Disabled>)>;
type LifecycleSurfaces<'w, 's> =
    Query<'w, 's, &'static WlSurfaceHandle, (With<Window>, Allow<Disabled>)>;
type LifecycleOutputs<'w, 's> = Query<'w, 's, (Entity, OutputRuntime)>;

#[derive(SystemParam)]
pub(crate) struct WindowLifecycleParams<'w, 's> {
    deferred_window_events: ResMut<'w, DeferredWindowEvents>,
    pending_popup_requests: ResMut<'w, PendingPopupServerRequests>,
    entity_index: Res<'w, EntityIndex>,
    pointer: Res<'w, GlobalPointerPosition>,
    active_grab: ResMut<'w, ActiveWindowGrab>,
    keyboard_focus: ResMut<'w, KeyboardFocusState>,
    focused_output: Res<'w, FocusedOutputState>,
    wayland_ingress: Res<'w, WaylandIngress>,
    windows: LifecycleWindows<'w, 's>,
    popups: LifecyclePopups<'w, 's>,
    existing_surfaces: LifecycleSurfaces<'w, 's>,
    outputs: LifecycleOutputs<'w, 's>,
    workspaces: Query<'w, 's, (Entity, WorkspaceRuntime)>,
    window_created: MessageWriter<'w, WindowCreated>,
    window_closed: MessageWriter<'w, WindowClosed>,
    window_moved: MessageWriter<'w, WindowMoved>,
}

pub(crate) fn window_lifecycle_system(
    mut commands: Commands,
    config: Res<CompositorConfig>,
    work_area: Res<WorkArea>,
    lifecycle: WindowLifecycleParams<'_, '_>,
) {
    let WindowLifecycleParams {
        mut deferred_window_events,
        mut pending_popup_requests,
        entity_index,
        pointer,
        mut active_grab,
        mut keyboard_focus,
        focused_output,
        wayland_ingress,
        mut windows,
        popups,
        existing_surfaces,
        outputs,
        workspaces,
        mut window_created,
        mut window_closed,
        mut window_moved,
    } = lifecycle;

    let primary_output_id = preferred_primary_output_id(Some(wayland_ingress.as_ref()));
    let active_workspace_entity = focused_or_primary_workspace_runtime_target(
        &outputs,
        focused_output.id,
        primary_output_id,
        &entity_index,
        1,
    );
    let mut requests = deferred_window_events.take();
    requests.extend(wayland_ingress.pending_window_events.iter().cloned());

    let mut known_surfaces = existing_surfaces.iter().map(|surface| surface.id).collect::<BTreeSet<_>>();
    let mut deferred = Vec::new();

    for request in requests {
        let result = match request.action.clone() {
            WindowEvent::Upsert { title, app_id, hints, scene_geometry, attached } => upsert_window(
                request.surface_id,
                title,
                app_id,
                hints,
                scene_geometry,
                attached,
                &config,
                &outputs,
                &workspaces,
                active_workspace_entity,
                &entity_index,
                &mut windows,
                &mut commands,
                &mut window_created,
                &mut window_moved,
                &mut known_surfaces,
            ),
            WindowEvent::Committed { size, attached } => apply_committed_window_state(
                request.surface_id,
                size,
                attached,
                &work_area,
                primary_output_id,
                &entity_index,
                &outputs,
                &workspaces,
                &mut windows,
                &mut commands,
            ),
            WindowEvent::ManagerRequest(manager_request) => apply_window_manager_request(
                request.surface_id,
                manager_request,
                &pointer,
                &entity_index,
                &mut windows,
                &mut active_grab,
                &mut keyboard_focus,
            ),
            WindowEvent::Closed => {
                destroy_window(
                    request.surface_id,
                    &entity_index,
                    &mut windows,
                    &popups,
                    &mut pending_popup_requests,
                    &mut commands,
                    &mut window_closed,
                );
                known_surfaces.remove(&request.surface_id);
                Ok(())
            }
        };

        if result.is_err() {
            deferred.push(request);
        }
    }

    deferred_window_events.replace(deferred);
}

fn upsert_window(
    surface_id: u64,
    title: Option<String>,
    app_id: Option<String>,
    hints: WindowManagementHints,
    scene_geometry: Option<WindowSceneGeometry>,
    attached: bool,
    config: &CompositorConfig,
    outputs: &Query<(Entity, OutputRuntime)>,
    _workspaces: &Query<(Entity, WorkspaceRuntime)>,
    active_workspace_entity: Option<Entity>,
    entity_index: &EntityIndex,
    windows: &mut LifecycleWindows<'_, '_>,
    commands: &mut Commands,
    window_created: &mut MessageWriter<WindowCreated>,
    window_moved: &mut MessageWriter<WindowMoved>,
    known_surfaces: &mut BTreeSet<u64>,
) -> Result<(), ()> {
    let Some(geometry) = scene_geometry.or_else(|| {
        (!known_surfaces.contains(&surface_id)).then_some(WindowSceneGeometry {
            x: 0,
            y: 0,
            width: 960,
            height: 720,
        })
    }) else {
        if let Some(entity) = resolve_window_entity(surface_id, entity_index, windows)
            && let Ok((entity, mut window)) = windows.get_mut(entity)
        {
            if let Some(title) = title {
                window.window.title = title;
            }
            if let Some(app_id) = app_id {
                window.window.app_id = app_id;
            }
            *window.management_hints = hints;
            refresh_window_policy_for_window(config, outputs, entity, &mut window, commands);
            return Ok(());
        }
        return Err(());
    };

    let resolved_title = title.unwrap_or_else(|| format!("Window {surface_id}"));
    let resolved_app_id = app_id.unwrap_or_default();
    let context = window_rule_context(&resolved_app_id, &resolved_title, &hints);
    let policy = config.resolve_window_policy_with_context(context);
    let background = config.resolve_window_background_with_context(context);

    if let Some(entity) = resolve_window_entity(surface_id, entity_index, windows)
        && let Ok((entity, mut window)) = windows.get_mut(entity)
    {
        let moved = window.scene_geometry.x != geometry.x || window.scene_geometry.y != geometry.y;
        let allow_geometry_update =
            !window.management_hints.client_driven_resize || window.content_version.value == 0;

        window.window.title = resolved_title.clone();
        window.window.app_id = resolved_app_id.clone();
        *window.management_hints = hints;
        if let Some(buffer) = window.buffer.as_mut() {
            buffer.attached = attached;
        }

        refresh_window_policy(
            policy,
            &mut window.layout,
            &mut window.mode,
            &mut window.restore,
            &mut window.policy_state,
        );
        let current_background = window.background.as_ref().map(|background| (*background).clone());
        let background_output_id = resolve_background_output_id(outputs, background.as_ref());
        sync_window_background_role(
            commands,
            entity,
            background_output_id,
            WindowBackgroundState::new(
                &mut window.role,
                &mut window.scene_geometry,
                &mut window.fullscreen_target,
                &mut window.layout,
                &mut window.mode,
            ),
            current_background,
        );

        if allow_geometry_update {
            window.geometry.x = geometry.x.clamp(i32::MIN as isize, i32::MAX as isize) as i32;
            window.geometry.y = geometry.y.clamp(i32::MIN as isize, i32::MAX as isize) as i32;
            window.geometry.width = geometry.width.max(1);
            window.geometry.height = geometry.height.max(1);
            *window.scene_geometry = WindowSceneGeometry {
                x: geometry.x,
                y: geometry.y,
                width: geometry.width.max(1),
                height: geometry.height.max(1),
            };
            if matches!(*window.layout, WindowLayout::Floating) && *window.mode == WindowMode::Normal
            {
                window.placement.set_explicit_position(WindowPosition {
                    x: geometry.x,
                    y: geometry.y,
                });
                window.placement.floating_size = Some(WindowSize {
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                });
            }
        }

        if moved {
            window_moved.write(WindowMoved {
                surface_id,
                x: geometry.x as i64,
                y: geometry.y as i64,
            });
        }
        if window.child_of.is_none() && let Some(workspace_entity) = active_workspace_entity {
            commands.entity(entity).insert(ChildOf(workspace_entity));
        }
        known_surfaces.insert(surface_id);
        return Ok(());
    }

    let mut placement = WindowPlacement::default();
    if hints.prefer_floating || matches!(policy.layout, WindowLayout::Floating) {
        placement.set_explicit_position(WindowPosition { x: geometry.x, y: geometry.y });
        placement.floating_size =
            Some(WindowSize { width: geometry.width.max(1), height: geometry.height.max(1) });
    }

    let window_entity = commands
        .spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: surface_id },
                geometry: SurfaceGeometry {
                    x: geometry.x.clamp(i32::MIN as isize, i32::MAX as isize) as i32,
                    y: geometry.y.clamp(i32::MIN as isize, i32::MAX as isize) as i32,
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                },
                scene_geometry: WindowSceneGeometry {
                    x: geometry.x,
                    y: geometry.y,
                    width: geometry.width.max(1),
                    height: geometry.height.max(1),
                },
                viewport_visibility: Default::default(),
                buffer: BufferState { attached, scale: 1 },
                content_version: SurfaceContentVersion::default(),
                window: Window {
                    app_id: resolved_app_id.clone(),
                    title: resolved_title.clone(),
                },
                management_hints: hints.clone(),
                layout: policy.layout,
                mode: policy.mode,
                decoration: ServerDecoration { enabled: true },
                border_theme: BorderTheme {
                    width: policy.layout.border_width(),
                    color: config.border_color.clone(),
                },
                animation: WindowAnimation::default(),
            },
            WindowPolicyState { applied: policy, locked: false },
            placement,
        ))
        .id();

    let mut scene_geometry = WindowSceneGeometry {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width.max(1),
        height: geometry.height.max(1),
    };
    let mut fullscreen_target = WindowFullscreenTarget::default();
    let mut role = WindowRole::Managed;
    let mut layout = policy.layout;
    let mut mode = policy.mode;
    let background_output_id = resolve_background_output_id(outputs, background.as_ref());
    sync_window_background_role(
        commands,
        window_entity,
        background_output_id,
        WindowBackgroundState::new(
            &mut role,
            &mut scene_geometry,
            &mut fullscreen_target,
            &mut layout,
            &mut mode,
        ),
        None,
    );
    commands
        .entity(window_entity)
        .insert((scene_geometry.clone(), fullscreen_target, layout, mode, role));
    if let Some(workspace_entity) = active_workspace_entity {
        commands.entity(window_entity).insert(ChildOf(workspace_entity));
    }
    known_surfaces.insert(surface_id);
    window_created.write(WindowCreated { surface_id, title: resolved_title });
    Ok(())
}

fn apply_committed_window_state(
    surface_id: u64,
    size: Option<SurfaceExtent>,
    attached: bool,
    work_area: &WorkArea,
    primary_output_id: Option<nekoland_ecs::components::OutputId>,
    entity_index: &EntityIndex,
    outputs: &LifecycleOutputs<'_, '_>,
    workspaces: &Query<(Entity, WorkspaceRuntime)>,
    windows: &mut LifecycleWindows<'_, '_>,
    commands: &mut Commands,
) -> Result<(), ()> {
    let entity = resolve_window_entity(surface_id, entity_index, windows).ok_or(())?;
    let (_, mut window) = windows.get_mut(entity).map_err(|_| ())?;
    if let Some(buffer) = window.buffer.as_mut() {
        buffer.attached = attached;
    }
    window.content_version.bump();

    let Some(size) = size else {
        return Ok(());
    };

    if !matches!(
        *window.mode,
        WindowMode::Maximized | WindowMode::Fullscreen | WindowMode::Hidden
    ) {
        let committed_width = size.width.max(1);
        let committed_height = size.height.max(1);
        if window.management_hints.client_driven_resize {
            let pending_resize_state = window.pending_resize.as_ref().map(|pending_resize| {
                (
                    pending_resize.requested_geometry.clone(),
                    pending_resize.inflight_geometry.clone(),
                    pending_resize.edges,
                )
            });
            if let Some((requested_geometry, inflight_geometry, edges)) = pending_resize_state {
                if let Some(inflight) = inflight_geometry {
                    let committed_geometry = anchored_resize_commit_geometry(
                        &inflight,
                        committed_width,
                        committed_height,
                        edges,
                    );
                    apply_committed_geometry(&mut window, &committed_geometry);
                    if requested_geometry == inflight {
                        commands.entity(entity).remove::<PendingInteractiveResize>();
                    } else if let Some(ref mut pending_resize) = window.pending_resize {
                        pending_resize.inflight_geometry = None;
                    }
                } else {
                    let committed_geometry = anchored_resize_commit_geometry(
                        &requested_geometry,
                        committed_width,
                        committed_height,
                        edges,
                    );
                    apply_committed_geometry(&mut window, &committed_geometry);
                    if requested_geometry.width == committed_width
                        && requested_geometry.height == committed_height
                    {
                        commands.entity(entity).remove::<PendingInteractiveResize>();
                    }
                }
            } else {
                window.scene_geometry.width = committed_width;
                window.scene_geometry.height = committed_height;
                if *window.layout == WindowLayout::Floating
                    && window.placement.floating_size.is_some()
                {
                    window.placement.floating_size =
                        Some(WindowSize { width: committed_width, height: committed_height });
                }
            }
        } else {
            window.scene_geometry.width = committed_width;
            window.scene_geometry.height = committed_height;
            if *window.layout == WindowLayout::Floating
                && window.placement.floating_size.is_some()
            {
                window.placement.floating_size =
                    Some(WindowSize { width: committed_width, height: committed_height });
            }
        }
    } else if let Some(restored) = window.restore.snapshot.as_mut() {
        restored.geometry.width = size.width.max(1);
        restored.geometry.height = size.height.max(1);
        if restored.layout == WindowLayout::Floating
            && should_auto_place_floating_window(&window.placement, &restored.geometry)
        {
            let workspace_id = nekoland_ecs::workspace_membership::window_workspace_runtime_id(
                window.child_of,
                workspaces,
            );
            let placement_area =
                placement_area_for_workspace(outputs, workspace_id, primary_output_id, work_area);
            restored.geometry.x = centre_x(&placement_area, 0, restored.geometry.width);
            restored.geometry.y = centre_y(&placement_area, 0, restored.geometry.height);
        }
    }

    Ok(())
}

fn apply_window_manager_request(
    surface_id: u64,
    request: WindowManagerRequest,
    pointer: &GlobalPointerPosition,
    entity_index: &EntityIndex,
    windows: &mut LifecycleWindows<'_, '_>,
    active_grab: &mut ActiveWindowGrab,
    keyboard_focus: &mut KeyboardFocusState,
) -> Result<(), ()> {
    let entity = resolve_window_entity(surface_id, entity_index, windows).ok_or(())?;
    let (_, mut window) = windows.get_mut(entity).map_err(|_| ())?;
    if window.role.is_output_background() {
        return Ok(());
    }

    match request {
        WindowManagerRequest::BeginMove => {
            if !window.management_hints.bypass_window_rules {
                *window.layout = WindowLayout::Floating;
                *window.mode = WindowMode::Normal;
                crate::window_policy::lock_window_policy(
                    *window.layout,
                    *window.mode,
                    &mut window.policy_state,
                );
            }
            keyboard_focus.focused_surface = Some(surface_id);
            begin_window_grab(
                active_grab,
                surface_id,
                WindowGrabMode::Move,
                pointer,
                &window.scene_geometry,
            );
        }
        WindowManagerRequest::BeginResize { edges } => {
            if !window.management_hints.bypass_window_rules {
                *window.layout = WindowLayout::Floating;
                *window.mode = WindowMode::Normal;
                crate::window_policy::lock_window_policy(
                    *window.layout,
                    *window.mode,
                    &mut window.policy_state,
                );
            }
            keyboard_focus.focused_surface = Some(surface_id);
            begin_window_grab(
                active_grab,
                surface_id,
                WindowGrabMode::Resize { edges },
                pointer,
                &window.scene_geometry,
            );
        }
        WindowManagerRequest::Maximize => {
            enter_temporary_window_mode(
                &window.scene_geometry,
                &mut window.fullscreen_target,
                &mut window.restore,
                *window.layout,
                &mut window.mode,
                WindowMode::Maximized,
                None,
            );
            keyboard_focus.focused_surface = Some(surface_id);
        }
        WindowManagerRequest::UnMaximize
        | WindowManagerRequest::UnFullscreen
        | WindowManagerRequest::UnMinimize => {
            restore_window_state(
                &window.policy_state,
                &mut window.scene_geometry,
                &mut window.fullscreen_target,
                &mut window.restore,
                &mut window.layout,
                &mut window.mode,
            );
        }
        WindowManagerRequest::Fullscreen { output_name } => {
            enter_temporary_window_mode(
                &window.scene_geometry,
                &mut window.fullscreen_target,
                &mut window.restore,
                *window.layout,
                &mut window.mode,
                WindowMode::Fullscreen,
                output_name.map(nekoland_ecs::selectors::OutputName::from),
            );
            keyboard_focus.focused_surface = Some(surface_id);
        }
        WindowManagerRequest::Minimize => {
            *window.mode = WindowMode::Hidden;
        }
    }

    Ok(())
}

fn destroy_window(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut LifecycleWindows<'_, '_>,
    popups: &LifecyclePopups<'_, '_>,
    pending_popup_requests: &mut PendingPopupServerRequests,
    commands: &mut Commands,
    window_closed: &mut MessageWriter<WindowClosed>,
) {
    let Some(entity) = resolve_window_entity(surface_id, entity_index, windows) else {
        return;
    };

    enqueue_popup_dismissals(surface_id, entity_index, popups, pending_popup_requests);
    commands.entity(entity).despawn();
    window_closed.write(WindowClosed { surface_id });
}

fn resolve_window_entity(
    surface_id: u64,
    entity_index: &EntityIndex,
    windows: &mut LifecycleWindows<'_, '_>,
) -> Option<Entity> {
    entity_index.entity_for_surface(surface_id).or_else(|| {
        windows
            .iter_mut()
            .find(|(_, window)| window.surface_id() == surface_id)
            .map(|(entity, _)| entity)
    })
}

fn refresh_window_policy_for_window(
    config: &CompositorConfig,
    outputs: &Query<(Entity, OutputRuntime)>,
    entity: Entity,
    window: &mut nekoland_ecs::views::WindowRuntimeItem<'_, '_>,
    commands: &mut Commands,
) {
    let context =
        window_rule_context(&window.window.app_id, &window.window.title, &window.management_hints);
    let policy = config.resolve_window_policy_with_context(context);
    let background = config.resolve_window_background_with_context(context);
    refresh_window_policy(
        policy,
        &mut window.layout,
        &mut window.mode,
        &mut window.restore,
        &mut window.policy_state,
    );
    let current_background = window.background.as_ref().map(|background| (*background).clone());
    let background_output_id = resolve_background_output_id(outputs, background.as_ref());
    sync_window_background_role(
        commands,
        entity,
        background_output_id,
        WindowBackgroundState::new(
            &mut window.role,
            &mut window.scene_geometry,
            &mut window.fullscreen_target,
            &mut window.layout,
            &mut window.mode,
        ),
        current_background,
    );
}

fn window_rule_context<'a>(
    app_id: &'a str,
    title: &'a str,
    hints: &'a WindowManagementHints,
) -> WindowRuleContext<'a> {
    WindowRuleContext {
        app_id,
        title,
        bypass_window_rules: hints.bypass_window_rules,
        helper_surface: hints.helper_surface,
        prefer_floating: hints.prefer_floating,
    }
}

fn anchored_resize_commit_geometry(
    target_geometry: &WindowSceneGeometry,
    committed_width: u32,
    committed_height: u32,
    edges: ResizeEdges,
) -> WindowSceneGeometry {
    let right = target_geometry.x.saturating_add(target_geometry.width as isize);
    let bottom = target_geometry.y.saturating_add(target_geometry.height as isize);
    let x = if edges.has_left() {
        right.saturating_sub(committed_width as isize)
    } else {
        target_geometry.x
    };
    let y = if edges.has_top() {
        bottom.saturating_sub(committed_height as isize)
    } else {
        target_geometry.y
    };
    WindowSceneGeometry { x, y, width: committed_width, height: committed_height }
}

fn apply_committed_geometry(
    window: &mut nekoland_ecs::views::WindowRuntimeItem<'_, '_>,
    committed_geometry: &WindowSceneGeometry,
) {
    window.scene_geometry.x = committed_geometry.x;
    window.scene_geometry.y = committed_geometry.y;
    window.scene_geometry.width = committed_geometry.width;
    window.scene_geometry.height = committed_geometry.height;
    window.placement.set_explicit_position(WindowPosition {
        x: committed_geometry.x,
        y: committed_geometry.y,
    });
    window.placement.floating_size = Some(WindowSize {
        width: committed_geometry.width,
        height: committed_geometry.height,
    });
}

fn placement_area_for_workspace(
    outputs: &Query<(Entity, OutputRuntime)>,
    workspace_id: Option<u32>,
    primary_output_id: Option<nekoland_ecs::components::OutputId>,
    work_area: &WorkArea,
) -> WorkArea {
    resolve_output_state_for_workspace(outputs, workspace_id, primary_output_id).map_or(
        *work_area,
        |(_, output, _, output_work_area)| {
            placement_work_area(
                &WorkArea {
                    x: output_work_area.x,
                    y: output_work_area.y,
                    width: output_work_area.width,
                    height: output_work_area.height,
                },
                Some(output),
            )
        },
    )
}

fn enqueue_popup_dismissals(
    parent_surface_id: u64,
    entity_index: &EntityIndex,
    popups: &LifecyclePopups<'_, '_>,
    pending_popup_requests: &mut PendingPopupServerRequests,
) {
    let Some(parent_entity) = entity_index.entity_for_surface(parent_surface_id) else {
        return;
    };
    let mut dismissed = BTreeSet::new();
    for popup in popups.iter() {
        if popup.child_of.parent() != parent_entity || !dismissed.insert(popup.surface_id()) {
            continue;
        }

        pending_popup_requests.push(PopupServerRequest {
            surface_id: popup.surface_id(),
            action: PopupServerAction::Dismiss,
        });
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{
        BufferState, PendingInteractiveResize, Window, WindowLayout, WindowManagementHints,
        WindowMode, WindowPlacement, WindowPosition, WindowSceneGeometry, WindowSize,
        WlSurfaceHandle, Workspace, WorkspaceId,
    };
    use nekoland_ecs::events::{WindowClosed, WindowCreated, WindowMoved};
    use nekoland_ecs::resources::{
        EntityIndex, FocusedOutputState, GlobalPointerPosition, KeyboardFocusState,
        PendingPopupServerRequests, SurfaceExtent, WaylandIngress, WindowEvent,
        WindowEventRequest, WorkArea, rebuild_entity_index_system,
    };

    use crate::interaction::ActiveWindowGrab;

    use super::{DeferredWindowEvents, window_lifecycle_system};

    #[test]
    fn upsert_creates_native_wayland_window_with_unified_window_component() {
        let mut app = NekolandApp::new("window-lifecycle-upsert-test");
        app.insert_resource(nekoland_config::resources::CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredWindowEvents::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(GlobalPointerPosition::default())
            .insert_resource(ActiveWindowGrab::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_message::<WindowMoved>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, window_lifecycle_system).chain(),
        );

        app.inner_mut().world_mut().spawn(Workspace {
            id: WorkspaceId(1),
            name: "1".to_owned(),
            active: true,
        });
        app.inner_mut().world_mut().resource_mut::<DeferredWindowEvents>().replace(vec![
            WindowEventRequest {
                surface_id: 41,
                action: WindowEvent::Upsert {
                    title: Some("Firefox".to_owned()),
                    app_id: Some("org.mozilla.firefox".to_owned()),
                    hints: WindowManagementHints::native_wayland(),
                    scene_geometry: Some(WindowSceneGeometry {
                        x: 0,
                        y: 0,
                        width: 800,
                        height: 600,
                    }),
                    attached: true,
                },
            },
        ]);

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        let mut windows = world.query::<(&WlSurfaceHandle, &Window, &WindowManagementHints)>();
        let Some((surface, window, hints)) = windows.iter(world).find(|(surface, _, _)| surface.id == 41) else {
            panic!("window should be created");
        };
        assert_eq!(window.title, "Firefox");
        assert_eq!(window.app_id, "org.mozilla.firefox");
        assert!(hints.client_driven_resize);
        assert!(!hints.helper_surface);
        assert_eq!(surface.id, 41);
    }

    #[test]
    fn committed_resize_reanchors_top_edge_for_client_driven_windows() {
        let mut app = NekolandApp::new("window-lifecycle-resize-anchor-test");
        app.insert_resource(nekoland_config::resources::CompositorConfig::default())
            .insert_resource(WorkArea::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(DeferredWindowEvents::default())
            .insert_resource(PendingPopupServerRequests::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(GlobalPointerPosition::default())
            .insert_resource(ActiveWindowGrab::default())
            .insert_resource(KeyboardFocusState::default())
            .insert_resource(WaylandIngress::default());
        app.inner_mut().add_message::<WindowCreated>().add_message::<WindowClosed>().add_message::<WindowMoved>().add_systems(
            LayoutSchedule,
            (rebuild_entity_index_system, window_lifecycle_system).chain(),
        );

        let entity = app
            .inner_mut()
            .world_mut()
            .spawn((
                WindowBundle {
                    surface: WlSurfaceHandle { id: 63 },
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 10,
                        y: 20,
                        width: 800,
                        height: 600,
                    },
                    scene_geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                    viewport_visibility: Default::default(),
                    buffer: BufferState { attached: true, scale: 1 },
                    content_version: Default::default(),
                    window: Window {
                        app_id: "org.mozilla.firefox".to_owned(),
                        title: "Firefox".to_owned(),
                    },
                    management_hints: WindowManagementHints::native_wayland(),
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    decoration: Default::default(),
                    border_theme: Default::default(),
                    animation: Default::default(),
                },
                PendingInteractiveResize {
                    requested_geometry: WindowSceneGeometry { x: 10, y: 120, width: 800, height: 500 },
                    inflight_geometry: Some(WindowSceneGeometry {
                        x: 10,
                        y: 120,
                        width: 800,
                        height: 500,
                    }),
                    edges: nekoland_ecs::resources::ResizeEdges::Top,
                },
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<DeferredWindowEvents>().replace(vec![
            WindowEventRequest {
                surface_id: 63,
                action: WindowEvent::Committed {
                    size: Some(SurfaceExtent { width: 800, height: 520 }),
                    attached: true,
                },
            },
        ]);

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let world = app.inner_mut().world_mut();
        assert_eq!(
            world.get::<WindowSceneGeometry>(entity),
            Some(&WindowSceneGeometry { x: 10, y: 100, width: 800, height: 520 })
        );
        assert_eq!(
            world.get::<WindowPlacement>(entity),
            Some(&WindowPlacement {
                floating_position: Some(nekoland_ecs::components::FloatingPosition::Explicit(
                    WindowPosition { x: 10, y: 100 },
                )),
                floating_size: Some(WindowSize { width: 800, height: 520 }),
            })
        );
    }
}
