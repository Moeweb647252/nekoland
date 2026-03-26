use std::collections::BTreeMap;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, Resource, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{OutputId, OutputViewport, Window};
use nekoland_ecs::control::{OutputOps, WindowOps};
use nekoland_ecs::resources::{
    EntityIndex, OverlayUiFrame, OverlayUiLayer, RenderColor, RenderRect, ShortcutRegistry,
    ShortcutState, ShortcutTrigger, WaylandIngress, WindowStackingState,
};
use nekoland_ecs::selectors::{OutputSelector, SurfaceId};
use nekoland_ecs::views::{OutputRuntime, WindowFocusRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::{
    active_workspace_runtime_target, window_workspace_runtime_id,
};

use crate::viewport::{preferred_primary_output_id, resolve_output_state_for_workspace};

const MAX_VISIBLE_SWITCHER_ITEMS: usize = 7;

/// Stable shortcut id for keeping the window switcher session open.
pub const WINDOW_SWITCHER_HOLD_SHORTCUT_ID: &str = "window_switcher.hold";
/// Stable shortcut id for moving to the next window switcher candidate.
pub const WINDOW_SWITCHER_CYCLE_NEXT_SHORTCUT_ID: &str = "window_switcher.cycle_next";
/// Stable shortcut id for moving to the previous window switcher candidate.
pub const WINDOW_SWITCHER_CYCLE_PREV_SHORTCUT_ID: &str = "window_switcher.cycle_prev";

/// Session state for the shell-local Alt+Tab style window switcher.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowSwitcherState {
    /// Whether a switcher session is currently active.
    pub active: bool,
    /// Workspace whose windows are currently being cycled.
    pub workspace_id: Option<u32>,
    /// Surface that held keyboard focus when the session began.
    pub anchor_surface: Option<SurfaceId>,
    /// Current candidate index within the front-to-back candidate list.
    pub selected_index: usize,
    /// Surface currently selected inside the switcher UI.
    pub selected_surface: Option<SurfaceId>,
    /// Surface most recently previewed through viewport centering.
    pub preview_surface: Option<SurfaceId>,
    /// Surface committed when the session ended on a different window.
    pub committed_surface: Option<SurfaceId>,
    /// Output whose viewport is used for preview and restore.
    pub origin_output_id: Option<OutputId>,
    /// Viewport origin captured when the session started.
    pub origin_viewport: Option<OutputViewport>,
}

impl WindowSwitcherState {
    fn begin_session(
        &mut self,
        workspace_id: u32,
        anchor_surface: Option<SurfaceId>,
        origin_output_id: OutputId,
        origin_viewport: OutputViewport,
    ) {
        self.active = true;
        self.workspace_id = Some(workspace_id);
        self.anchor_surface = anchor_surface;
        self.selected_index = 0;
        self.selected_surface = None;
        self.preview_surface = None;
        self.committed_surface = None;
        self.origin_output_id = Some(origin_output_id);
        self.origin_viewport = Some(origin_viewport);
    }

    fn end_session(&mut self) {
        self.active = false;
        self.workspace_id = None;
        self.anchor_surface = None;
        self.selected_index = 0;
        self.selected_surface = None;
        self.preview_surface = None;
        self.origin_output_id = None;
        self.origin_viewport = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowSwitcherCandidate {
    surface_id: SurfaceId,
    title: String,
    app_id: String,
    width: u32,
    height: u32,
}

struct SwitcherContext {
    workspace_id: u32,
    output_id: OutputId,
    viewport: OutputViewport,
}

type SwitcherWindows<'w, 's> = Query<'w, 's, WindowFocusRuntime, (With<Window>, Allow<Disabled>)>;
type SwitcherWorkspaces<'w, 's> = Query<'w, 's, (Entity, WorkspaceRuntime), Allow<Disabled>>;
type SwitcherOutputs<'w, 's> = Query<'w, 's, (Entity, OutputRuntime)>;

/// Drives switcher session lifecycle directly from raw pressed-key state.
pub fn window_switcher_input_system(
    shortcuts: Res<'_, ShortcutState>,
    wayland_ingress: Res<'_, WaylandIngress>,
    entity_index: Res<'_, EntityIndex>,
    stacking: Res<'_, WindowStackingState>,
    mut switcher: ResMut<'_, WindowSwitcherState>,
    mut window_ops: WindowOps<'_, '_>,
    mut output_ops: OutputOps<'_, '_>,
    windows: SwitcherWindows<'_, '_>,
    workspaces: SwitcherWorkspaces<'_, '_>,
    outputs: SwitcherOutputs<'_, '_>,
) {
    let hold_active = shortcuts.active(WINDOW_SWITCHER_HOLD_SHORTCUT_ID);
    let next_pressed = shortcuts.just_pressed(WINDOW_SWITCHER_CYCLE_NEXT_SHORTCUT_ID);
    let prev_pressed = shortcuts.just_pressed(WINDOW_SWITCHER_CYCLE_PREV_SHORTCUT_ID);
    let reverse = prev_pressed;

    if switcher.active {
        if next_pressed || prev_pressed {
            advance_window_switcher_selection(
                reverse,
                &stacking,
                &windows,
                &workspaces,
                &mut switcher,
                &mut output_ops,
            );
        }

        if !hold_active {
            finalize_window_switcher_session(&mut switcher, &mut window_ops, &mut output_ops);
        }
        return;
    }

    if !(next_pressed || prev_pressed) || !hold_active {
        return;
    }

    let anchor_surface = window_ops.focused_surface_id();
    let Some(context) = resolve_switcher_context(
        anchor_surface,
        &entity_index,
        &wayland_ingress,
        &windows,
        &workspaces,
        &outputs,
    ) else {
        return;
    };

    let candidates =
        switcher_candidates(context.workspace_id, anchor_surface, &stacking, &windows, &workspaces);
    if candidates.is_empty() {
        return;
    }

    switcher.begin_session(
        context.workspace_id,
        anchor_surface,
        context.output_id,
        context.viewport,
    );
    let next_index = next_candidate_index(anchor_surface, None, &candidates, reverse);
    apply_switcher_selection(next_index, &candidates, &mut switcher, &mut output_ops);
}

/// Registers window-switcher-owned shortcuts into the global shortcut registry.
pub fn register_shortcuts(registry: &mut ShortcutRegistry) {
    registry
        .register(nekoland_ecs::resources::ShortcutSpec::new(
            WINDOW_SWITCHER_HOLD_SHORTCUT_ID,
            "window_switcher",
            "Hold while cycling the window switcher",
            "Alt",
            ShortcutTrigger::Hold,
        ))
        .expect("window switcher shortcut ids should be unique");
    registry
        .register(nekoland_ecs::resources::ShortcutSpec::new(
            WINDOW_SWITCHER_CYCLE_NEXT_SHORTCUT_ID,
            "window_switcher",
            "Cycle to the next switcher candidate",
            "Alt+Tab",
            ShortcutTrigger::Press,
        ))
        .expect("window switcher shortcut ids should be unique");
    registry
        .register(nekoland_ecs::resources::ShortcutSpec::new(
            WINDOW_SWITCHER_CYCLE_PREV_SHORTCUT_ID,
            "window_switcher",
            "Cycle to the previous switcher candidate",
            "Alt+Shift+Tab",
            ShortcutTrigger::Press,
        ))
        .expect("window switcher shortcut ids should be unique");
}

/// Emits a simple output-local overlay for the current switcher session.
pub fn window_switcher_overlay_system(
    switcher: Res<'_, WindowSwitcherState>,
    mut overlay_ui: ResMut<'_, OverlayUiFrame>,
    windows: SwitcherWindows<'_, '_>,
    workspaces: SwitcherWorkspaces<'_, '_>,
    outputs: SwitcherOutputs<'_, '_>,
    stacking: Res<'_, WindowStackingState>,
) {
    if !switcher.active {
        return;
    }

    let Some(workspace_id) = switcher.workspace_id else {
        return;
    };
    let Some(output_id) = switcher.origin_output_id else {
        return;
    };
    let Some((_, output)) = outputs.iter().find(|(_, output)| output.id() == output_id) else {
        return;
    };

    let candidates = switcher_candidates(
        workspace_id,
        switcher.anchor_surface,
        &stacking,
        &windows,
        &workspaces,
    );
    if candidates.is_empty() {
        return;
    }

    let selected_index = candidates
        .iter()
        .position(|candidate| Some(candidate.surface_id) == switcher.selected_surface)
        .unwrap_or_else(|| switcher.selected_index.min(candidates.len().saturating_sub(1)));
    let (start, end) = visible_candidate_window(candidates.len(), selected_index);
    let visible = &candidates[start..end];

    let row_height = 70_i32;
    let row_spacing = 10_i32;
    let padding = 18_i32;
    let header_height = 28_i32;
    let thumbnail_width = 96_u32;
    let thumbnail_height = 54_u32;
    let panel_width = output.properties.width.saturating_sub(120).min(760).max(420);
    let content_rows = visible.len() as i32;
    let panel_height = (padding * 2
        + header_height
        + content_rows * row_height
        + (content_rows.saturating_sub(1) * row_spacing))
        .max(120) as u32;
    let panel_x = ((output.properties.width.saturating_sub(panel_width)) / 2) as i32;
    let panel_y = ((output.properties.height.saturating_sub(panel_height)) / 3).max(24) as i32;
    let panel_rect =
        RenderRect { x: panel_x, y: panel_y, width: panel_width, height: panel_height };
    let mut output_overlay = overlay_ui.output(output_id);

    output_overlay
        .panel(
            "window_switcher.backdrop",
            OverlayUiLayer::Foreground,
            panel_rect,
            None,
            RenderColor { r: 20, g: 24, b: 33, a: 255 },
            0.9,
            10,
        )
        .text(
            "window_switcher.header",
            OverlayUiLayer::Foreground,
            panel_x + padding,
            panel_y + padding,
            Some(panel_rect),
            "Switch Windows",
            18.0,
            RenderColor { r: 235, g: 239, b: 247, a: 255 },
            1.0,
            20,
        );

    let mut cursor_y = panel_y + padding + header_height;
    for candidate in visible {
        let item_rect = RenderRect {
            x: panel_x + 10,
            y: cursor_y - 4,
            width: panel_width.saturating_sub(20),
            height: row_height as u32,
        };
        if Some(candidate.surface_id) == switcher.selected_surface {
            output_overlay.panel(
                format!("window_switcher.item.{}", candidate.surface_id.0),
                OverlayUiLayer::Foreground,
                item_rect,
                Some(panel_rect),
                RenderColor { r: 76, g: 110, b: 245, a: 255 },
                0.85,
                15,
            );
        }
        let thumbnail_frame = RenderRect {
            x: item_rect.x + 12,
            y: item_rect.y + ((row_height - thumbnail_height as i32) / 2),
            width: thumbnail_width,
            height: thumbnail_height,
        };
        output_overlay.panel(
            format!("window_switcher.thumb_frame.{}", candidate.surface_id.0),
            OverlayUiLayer::Foreground,
            thumbnail_frame,
            Some(panel_rect),
            RenderColor { r: 11, g: 15, b: 22, a: 255 },
            0.95,
            18,
        );
        output_overlay.surface(
            format!("window_switcher.thumb.{}", candidate.surface_id.0),
            OverlayUiLayer::Foreground,
            candidate.surface_id.0,
            fit_rect_within(thumbnail_frame, candidate.width.max(1), candidate.height.max(1)),
            None,
            1.0,
            19,
        );
        let title_x = thumbnail_frame.x + thumbnail_frame.width as i32 + 16;
        let title_clip = Some(panel_rect);
        output_overlay.text(
            format!("window_switcher.title.{}", candidate.surface_id.0),
            OverlayUiLayer::Foreground,
            title_x,
            item_rect.y + 18,
            title_clip,
            candidate_title(candidate),
            15.0,
            RenderColor { r: 247, g: 250, b: 255, a: 255 },
            1.0,
            25,
        );
        if let Some(app_label) = candidate_app_label(candidate) {
            output_overlay.text(
                format!("window_switcher.app.{}", candidate.surface_id.0),
                OverlayUiLayer::Foreground,
                title_x,
                item_rect.y + 40,
                title_clip,
                app_label,
                12.0,
                RenderColor { r: 173, g: 181, b: 197, a: 255 },
                1.0,
                24,
            );
        }
        cursor_y += row_height + row_spacing;
    }
}

fn advance_window_switcher_selection(
    reverse: bool,
    stacking: &WindowStackingState,
    windows: &SwitcherWindows<'_, '_>,
    workspaces: &SwitcherWorkspaces<'_, '_>,
    switcher: &mut WindowSwitcherState,
    output_ops: &mut OutputOps<'_, '_>,
) {
    let Some(workspace_id) = switcher.workspace_id else {
        switcher.end_session();
        return;
    };

    let candidates =
        switcher_candidates(workspace_id, switcher.anchor_surface, stacking, windows, workspaces);
    if candidates.is_empty() {
        switcher.end_session();
        return;
    }

    let next_index = next_candidate_index(
        switcher.anchor_surface,
        switcher.selected_surface,
        &candidates,
        reverse,
    );
    apply_switcher_selection(next_index, &candidates, switcher, output_ops);
}

fn apply_switcher_selection(
    next_index: usize,
    candidates: &[WindowSwitcherCandidate],
    switcher: &mut WindowSwitcherState,
    output_ops: &mut OutputOps<'_, '_>,
) {
    let Some(candidate) = candidates.get(next_index) else {
        return;
    };
    let selected_surface = candidate.surface_id;
    switcher.selected_index = next_index;
    switcher.selected_surface = Some(selected_surface);

    if switcher.preview_surface == Some(selected_surface) {
        return;
    }

    if let Some(output_id) = switcher.origin_output_id {
        output_ops
            .select(OutputSelector::Id(output_id))
            .center_viewport_on_window(selected_surface);
    }
    switcher.preview_surface = Some(selected_surface);
}

fn finalize_window_switcher_session(
    switcher: &mut WindowSwitcherState,
    window_ops: &mut WindowOps<'_, '_>,
    output_ops: &mut OutputOps<'_, '_>,
) {
    let final_selected = switcher.selected_surface.or(switcher.anchor_surface);
    if let Some(surface_id) = final_selected
        && Some(surface_id) != switcher.anchor_surface
    {
        window_ops.surface(surface_id).focus();
        switcher.committed_surface = Some(surface_id);
    } else if let (Some(output_id), Some(origin_viewport)) =
        (switcher.origin_output_id, switcher.origin_viewport.clone())
    {
        output_ops
            .select(OutputSelector::Id(output_id))
            .move_viewport_to(origin_viewport.origin_x, origin_viewport.origin_y);
    }

    switcher.end_session();
}

fn resolve_switcher_context(
    anchor_surface: Option<SurfaceId>,
    entity_index: &EntityIndex,
    wayland_ingress: &WaylandIngress,
    windows: &SwitcherWindows<'_, '_>,
    workspaces: &SwitcherWorkspaces<'_, '_>,
    outputs: &SwitcherOutputs<'_, '_>,
) -> Option<SwitcherContext> {
    let workspace_id = anchor_surface
        .and_then(|surface_id| entity_index.entity_for_surface(surface_id.0))
        .and_then(|entity| windows.get(entity).ok())
        .and_then(|window| window_workspace_runtime_id(window.child_of, workspaces))
        .or_else(|| active_workspace_runtime_target(workspaces).1)?;
    let primary_output_id = preferred_primary_output_id(Some(wayland_ingress));
    let (output_id, _, viewport, _) =
        resolve_output_state_for_workspace(outputs, Some(workspace_id), primary_output_id)?;

    Some(SwitcherContext { workspace_id, output_id, viewport: viewport.clone() })
}

fn switcher_candidates(
    workspace_id: u32,
    _anchor_surface: Option<SurfaceId>,
    stacking: &WindowStackingState,
    windows: &SwitcherWindows<'_, '_>,
    workspaces: &SwitcherWorkspaces<'_, '_>,
) -> Vec<WindowSwitcherCandidate> {
    let mut by_surface = BTreeMap::<u64, WindowSwitcherCandidate>::new();
    for window in windows.iter() {
        if *window.mode == nekoland_ecs::components::WindowMode::Hidden
            || !window.role.is_managed()
            || window.management_hints.helper_surface
        {
            continue;
        }

        if window_workspace_runtime_id(window.child_of, workspaces) != Some(workspace_id) {
            continue;
        }

        by_surface.insert(
            window.surface_id(),
            WindowSwitcherCandidate {
                surface_id: SurfaceId(window.surface_id()),
                title: window.window.title.clone(),
                app_id: window.window.app_id.clone(),
                width: window.geometry.width.max(1),
                height: window.geometry.height.max(1),
            },
        );
    }

    let ordered_inputs = by_surface.keys().copied().map(|surface_id| (workspace_id, surface_id));
    let mut ordered = stacking.ordered_surfaces(ordered_inputs);
    ordered.reverse();

    ordered.into_iter().filter_map(|surface_id| by_surface.remove(&surface_id)).collect()
}

fn next_candidate_index(
    anchor_surface: Option<SurfaceId>,
    current_selected: Option<SurfaceId>,
    candidates: &[WindowSwitcherCandidate],
    reverse: bool,
) -> usize {
    let candidate_count = candidates.len();
    if candidate_count == 0 {
        return 0;
    }

    let current_index = current_selected.or(anchor_surface).and_then(|surface_id| {
        candidates.iter().position(|candidate| candidate.surface_id == surface_id)
    });

    if current_selected.is_none() {
        return match current_index {
            Some(index) if reverse => index.checked_sub(1).unwrap_or(candidate_count - 1),
            Some(index) => (index + 1) % candidate_count,
            None if reverse => candidate_count - 1,
            None => 0,
        };
    }

    match current_index {
        Some(index) if reverse => index.checked_sub(1).unwrap_or(candidate_count - 1),
        Some(index) => (index + 1) % candidate_count,
        None if reverse => candidate_count - 1,
        None => 0,
    }
}

fn visible_candidate_window(candidate_count: usize, selected_index: usize) -> (usize, usize) {
    if candidate_count <= MAX_VISIBLE_SWITCHER_ITEMS {
        return (0, candidate_count);
    }

    let half = MAX_VISIBLE_SWITCHER_ITEMS / 2;
    let mut start = selected_index.saturating_sub(half);
    let mut end = start + MAX_VISIBLE_SWITCHER_ITEMS;
    if end > candidate_count {
        end = candidate_count;
        start = end - MAX_VISIBLE_SWITCHER_ITEMS;
    }
    (start, end)
}

fn candidate_title(candidate: &WindowSwitcherCandidate) -> String {
    if candidate.title.is_empty() {
        format!("Window {}", candidate.surface_id.0)
    } else {
        candidate.title.clone()
    }
}

fn candidate_app_label(candidate: &WindowSwitcherCandidate) -> Option<String> {
    (!candidate.app_id.is_empty()).then(|| candidate.app_id.clone())
}

fn fit_rect_within(frame: RenderRect, content_width: u32, content_height: u32) -> RenderRect {
    if frame.is_empty() {
        return frame;
    }

    let content_width = content_width.max(1);
    let content_height = content_height.max(1);
    let width_limited_height =
        ((u64::from(frame.width) * u64::from(content_height)) / u64::from(content_width)) as u32;
    let (fitted_width, fitted_height) = if width_limited_height <= frame.height {
        (frame.width, width_limited_height.max(1))
    } else {
        let height_limited_width = ((u64::from(frame.height) * u64::from(content_width))
            / u64::from(content_height)) as u32;
        (height_limited_width.max(1), frame.height)
    };

    RenderRect {
        x: frame.x + ((frame.width as i32 - fitted_width as i32) / 2),
        y: frame.y + ((frame.height as i32 - fitted_height as i32) / 2),
        width: fitted_width,
        height: fitted_height,
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::World;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        OutputCurrentWorkspace, OutputDevice, OutputId, OutputKind, OutputProperties,
        OutputViewport, WindowLayout, WindowMode, WindowSceneGeometry, WlSurfaceHandle, Workspace,
        WorkspaceId,
    };
    use nekoland_ecs::resources::{
        KeyboardFocusState, OverlayUiFrame, OverlayUiPrimitive, PendingOutputControls,
        PendingWindowControls, WaylandIngress, WindowStackingState, register_entity_index_hooks,
    };
    use nekoland_ecs::selectors::{OutputSelector, SurfaceId};

    use super::{
        WindowSwitcherState, window_switcher_input_system, window_switcher_overlay_system,
    };

    fn build_switcher_test_app() -> (NekolandApp, OutputId) {
        let mut app = NekolandApp::new("window-switcher-test");
        register_entity_index_hooks(app.inner_mut().world_mut());
        app.inner_mut()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<nekoland_ecs::resources::ShortcutState>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingWindowControls>()
            .init_resource::<WindowStackingState>()
            .init_resource::<WaylandIngress>()
            .init_resource::<OverlayUiFrame>()
            .init_resource::<WindowSwitcherState>()
            .add_systems(
                LayoutSchedule,
                (window_switcher_input_system, window_switcher_overlay_system).chain(),
            );

        let workspace = app
            .inner_mut()
            .world_mut()
            .spawn(Workspace { id: WorkspaceId(1), name: "1".to_owned(), active: true })
            .id();
        let output_id = OutputId(7);
        app.inner_mut().world_mut().spawn((
            output_id,
            OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Virtual".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                viewport: OutputViewport { origin_x: 100, origin_y: 200 },
                ..Default::default()
            },
            OutputCurrentWorkspace { workspace: WorkspaceId(1) },
        ));

        spawn_window(app.inner_mut().world_mut(), workspace, 11, "back");
        spawn_window(app.inner_mut().world_mut(), workspace, 22, "front");
        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface = Some(22);
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(output_id);
        app.inner_mut()
            .world_mut()
            .resource_mut::<WindowStackingState>()
            .workspaces
            .insert(1, vec![11, 22]);
        (app, output_id)
    }

    fn spawn_window(
        world: &mut World,
        workspace: bevy_ecs::prelude::Entity,
        surface_id: u64,
        title: &str,
    ) {
        world.spawn((
            WindowBundle {
                surface: WlSurfaceHandle { id: surface_id },
                geometry: nekoland_ecs::components::SurfaceGeometry {
                    x: surface_id as i32,
                    y: 0,
                    width: 200,
                    height: 160,
                },
                scene_geometry: WindowSceneGeometry {
                    x: surface_id as isize,
                    y: 0,
                    width: 200,
                    height: 160,
                },
                window: nekoland_ecs::components::Window {
                    app_id: format!("app.{surface_id}"),
                    title: title.to_owned(),
                },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                ..Default::default()
            },
            ChildOf(workspace),
        ));
    }

    fn press_alt_tab(world: &mut World) {
        let mut shortcuts = world.resource_mut::<nekoland_ecs::resources::ShortcutState>();
        shortcuts.set(super::WINDOW_SWITCHER_HOLD_SHORTCUT_ID, true, true, false);
        shortcuts.set(super::WINDOW_SWITCHER_CYCLE_NEXT_SHORTCUT_ID, true, true, false);
        shortcuts.set(super::WINDOW_SWITCHER_CYCLE_PREV_SHORTCUT_ID, false, false, false);
    }

    fn press_alt_shift_tab(world: &mut World) {
        let mut shortcuts = world.resource_mut::<nekoland_ecs::resources::ShortcutState>();
        shortcuts.set(super::WINDOW_SWITCHER_HOLD_SHORTCUT_ID, true, false, false);
        shortcuts.set(super::WINDOW_SWITCHER_CYCLE_NEXT_SHORTCUT_ID, false, false, false);
        shortcuts.set(super::WINDOW_SWITCHER_CYCLE_PREV_SHORTCUT_ID, true, true, false);
    }

    fn release_alt(world: &mut World) {
        let mut shortcuts = world.resource_mut::<nekoland_ecs::resources::ShortcutState>();
        shortcuts.set(super::WINDOW_SWITCHER_HOLD_SHORTCUT_ID, false, false, true);
        shortcuts.set(super::WINDOW_SWITCHER_CYCLE_NEXT_SHORTCUT_ID, false, false, false);
        shortcuts.set(super::WINDOW_SWITCHER_CYCLE_PREV_SHORTCUT_ID, false, false, false);
    }

    #[test]
    fn alt_tab_starts_session_and_queues_preview_for_next_window() {
        let (mut app, output_id) = build_switcher_test_app();
        press_alt_tab(app.inner_mut().world_mut());

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let state = app.inner().world().resource::<WindowSwitcherState>();
        assert!(state.active);
        assert_eq!(state.anchor_surface, Some(SurfaceId(22)));
        assert_eq!(state.selected_surface, Some(SurfaceId(11)));
        assert_eq!(state.origin_output_id, Some(output_id));
        assert_eq!(state.origin_viewport, Some(OutputViewport { origin_x: 100, origin_y: 200 }));

        let controls = app.inner().world().resource::<PendingOutputControls>();
        assert_eq!(controls.as_slice().len(), 1);
        assert_eq!(controls.as_slice()[0].selector, OutputSelector::Id(output_id));
        assert_eq!(controls.as_slice()[0].center_viewport_on, Some(SurfaceId(11)));
    }

    #[test]
    fn releasing_alt_focuses_the_newly_selected_window() {
        let (mut app, _) = build_switcher_test_app();
        press_alt_tab(app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        release_alt(app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let state = app.inner().world().resource::<WindowSwitcherState>();
        assert!(!state.active);

        let controls = app.inner().world().resource::<PendingWindowControls>();
        assert_eq!(controls.as_slice().len(), 1);
        assert_eq!(controls.as_slice()[0].surface_id, SurfaceId(11));
        assert!(controls.as_slice()[0].focus);
    }

    #[test]
    fn returning_to_anchor_restores_the_original_viewport_on_release() {
        let (mut app, output_id) = build_switcher_test_app();
        press_alt_tab(app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);
        app.inner_mut().world_mut().resource_mut::<PendingOutputControls>().clear();

        press_alt_shift_tab(app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        {
            let state = app.inner().world().resource::<WindowSwitcherState>();
            assert!(state.active);
            assert_eq!(state.selected_surface, Some(SurfaceId(22)));
        }
        app.inner_mut().world_mut().resource_mut::<PendingOutputControls>().clear();

        release_alt(app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let state = app.inner().world().resource::<WindowSwitcherState>();
        assert!(!state.active);
        let controls = app.inner().world().resource::<PendingOutputControls>();
        assert_eq!(controls.as_slice().len(), 1);
        assert_eq!(controls.as_slice()[0].selector, OutputSelector::Id(output_id));
        assert_eq!(
            controls.as_slice()[0].viewport_origin,
            Some(nekoland_ecs::resources::OutputViewportOrigin { x: 100, y: 200 })
        );
        assert!(controls.as_slice()[0].center_viewport_on.is_none());
    }

    #[test]
    fn switcher_overlay_emits_live_surface_thumbnails() {
        let (mut app, output_id) = build_switcher_test_app();
        press_alt_tab(app.inner_mut().world_mut());

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let overlay = app
            .inner()
            .world()
            .resource::<OverlayUiFrame>()
            .outputs
            .get(&output_id)
            .expect("switcher overlay should target the origin output");
        assert!(overlay.primitives.iter().any(|primitive| {
            matches!(
                primitive,
                OverlayUiPrimitive::Surface(surface) if surface.surface_id == 11
            )
        }));
    }
}
