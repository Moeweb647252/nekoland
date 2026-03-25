use std::collections::BTreeMap;

use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::{Entity, Query, Res, ResMut, Resource, With};
use bevy_ecs::query::Allow;
use nekoland_ecs::components::{OutputId, OutputViewport, Window};
use nekoland_ecs::control::{OutputOps, WindowOps};
use nekoland_ecs::resources::{
    EntityIndex, ModifierMask, OverlayUiFrame, OverlayUiLayer, PressedKeys, RenderColor,
    RenderRect, WaylandIngress, WindowStackingState,
};
use nekoland_ecs::selectors::{OutputSelector, SurfaceId};
use nekoland_ecs::views::{OutputRuntime, WindowFocusRuntime, WorkspaceRuntime};
use nekoland_ecs::workspace_membership::{
    active_workspace_runtime_target, window_workspace_runtime_id,
};

use crate::viewport::{preferred_primary_output_id, resolve_output_state_for_workspace};

const TAB_KEYCODE: u32 = 23;
const ALT_REQUIRED_MODIFIERS: ModifierMask = ModifierMask::new(false, true, false, false);
const SWITCHER_PANEL_SCREEN_MARGIN: u32 = 24;
const SWITCHER_PANEL_PADDING: u32 = 18;
const SWITCHER_HEADER_HEIGHT: u32 = 28;
const SWITCHER_HEADER_GRID_GAP: u32 = 16;
const SWITCHER_CARD_WIDTH: u32 = 220;
const SWITCHER_CARD_HEIGHT: u32 = 176;
const SWITCHER_CARD_GAP_X: u32 = 12;
const SWITCHER_CARD_GAP_Y: u32 = 16;
const SWITCHER_CARD_PADDING: u32 = 12;
const SWITCHER_CARD_TITLE_HEIGHT: u32 = 34;
const SWITCHER_CARD_TITLE_LINE_HEIGHT: i32 = 16;
const SWITCHER_CARD_TITLE_FONT_SIZE: f32 = 13.0;
const SWITCHER_CARD_TITLE_GAP: u32 = 8;
const SWITCHER_CARD_TITLE_MAX_UNITS: usize = 30;
const SWITCHER_THUMBNAIL_FRAME_WIDTH: u32 = 196;
const SWITCHER_THUMBNAIL_FRAME_HEIGHT: u32 = 110;

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
    width: u32,
    height: u32,
}

struct SwitcherContext {
    workspace_id: u32,
    output_id: OutputId,
    viewport: OutputViewport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SwitcherGridLayout {
    panel_rect: RenderRect,
    columns: usize,
    visible_capacity: usize,
    grid_origin_x: i32,
    grid_origin_y: i32,
}

impl SwitcherGridLayout {
    fn card_rect(self, slot_index: usize) -> RenderRect {
        let column = slot_index % self.columns;
        let row = slot_index / self.columns;
        let stride_x = (SWITCHER_CARD_WIDTH + SWITCHER_CARD_GAP_X) as i32;
        let stride_y = (SWITCHER_CARD_HEIGHT + SWITCHER_CARD_GAP_Y) as i32;

        RenderRect {
            x: self.grid_origin_x + column as i32 * stride_x,
            y: self.grid_origin_y + row as i32 * stride_y,
            width: SWITCHER_CARD_WIDTH,
            height: SWITCHER_CARD_HEIGHT,
        }
    }
}

type SwitcherWindows<'w, 's> = Query<'w, 's, WindowFocusRuntime, (With<Window>, Allow<Disabled>)>;
type SwitcherWorkspaces<'w, 's> = Query<'w, 's, (Entity, WorkspaceRuntime), Allow<Disabled>>;
type SwitcherOutputs<'w, 's> = Query<'w, 's, (Entity, OutputRuntime)>;

/// Drives switcher session lifecycle directly from raw pressed-key state.
pub fn window_switcher_input_system(
    pressed_keys: Res<'_, PressedKeys>,
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
    let alt_held = ALT_REQUIRED_MODIFIERS.matches_required(pressed_keys.modifiers());
    let tab_pressed = pressed_keys.was_key_just_pressed(TAB_KEYCODE);
    let reverse = pressed_keys.modifiers().shift;

    if switcher.active {
        if tab_pressed && alt_held {
            advance_window_switcher_selection(
                reverse,
                &stacking,
                &windows,
                &workspaces,
                &mut switcher,
                &mut output_ops,
            );
        }

        if !alt_held {
            finalize_window_switcher_session(&mut switcher, &mut window_ops, &mut output_ops);
        }
        return;
    }

    if !tab_pressed || !alt_held {
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
    let Some(layout) =
        switcher_grid_layout(output.properties.width, output.properties.height, candidates.len())
    else {
        return;
    };
    let (start, end) =
        visible_candidate_page(candidates.len(), selected_index, layout.visible_capacity);
    let visible = &candidates[start..end];

    let panel_rect = layout.panel_rect;
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
            panel_rect.x + SWITCHER_PANEL_PADDING as i32,
            panel_rect.y + SWITCHER_PANEL_PADDING as i32,
            Some(panel_rect),
            "Switch Windows",
            18.0,
            RenderColor { r: 235, g: 239, b: 247, a: 255 },
            1.0,
            20,
        );

    for (slot_index, candidate) in visible.iter().enumerate() {
        let item_rect = layout.card_rect(slot_index);
        let selected = Some(candidate.surface_id) == switcher.selected_surface;
        output_overlay.panel(
            format!("window_switcher.item.{}", candidate.surface_id.0),
            OverlayUiLayer::Foreground,
            item_rect,
            Some(panel_rect),
            if selected {
                RenderColor { r: 76, g: 110, b: 245, a: 255 }
            } else {
                RenderColor { r: 30, g: 36, b: 48, a: 255 }
            },
            if selected { 0.85 } else { 0.72 },
            if selected { 15 } else { 14 },
        );

        let title_rect = RenderRect {
            x: item_rect.x + SWITCHER_CARD_PADDING as i32,
            y: item_rect.y + SWITCHER_CARD_PADDING as i32,
            width: item_rect.width.saturating_sub(SWITCHER_CARD_PADDING * 2),
            height: SWITCHER_CARD_TITLE_HEIGHT,
        };
        for (line_index, line) in
            wrap_switcher_title(&candidate_title(candidate), SWITCHER_CARD_TITLE_MAX_UNITS)
                .into_iter()
                .enumerate()
        {
            output_overlay.text(
                format!("window_switcher.title.{}.{}", candidate.surface_id.0, line_index),
                OverlayUiLayer::Foreground,
                title_rect.x,
                title_rect.y + line_index as i32 * SWITCHER_CARD_TITLE_LINE_HEIGHT,
                Some(title_rect),
                line,
                SWITCHER_CARD_TITLE_FONT_SIZE,
                RenderColor { r: 247, g: 250, b: 255, a: 255 },
                1.0,
                25,
            );
        }

        let thumbnail_frame = RenderRect {
            x: item_rect.x + SWITCHER_CARD_PADDING as i32,
            y: item_rect.y
                + SWITCHER_CARD_PADDING as i32
                + SWITCHER_CARD_TITLE_HEIGHT as i32
                + SWITCHER_CARD_TITLE_GAP as i32,
            width: SWITCHER_THUMBNAIL_FRAME_WIDTH,
            height: SWITCHER_THUMBNAIL_FRAME_HEIGHT,
        };
        output_overlay.panel(
            format!("window_switcher.thumb_frame.{}", candidate.surface_id.0),
            OverlayUiLayer::Foreground,
            thumbnail_frame,
            Some(item_rect),
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

fn visible_candidate_page(
    candidate_count: usize,
    selected_index: usize,
    visible_capacity: usize,
) -> (usize, usize) {
    if candidate_count == 0 {
        return (0, 0);
    }

    let visible_capacity = visible_capacity.max(1);
    let start = selected_index / visible_capacity * visible_capacity;
    let end = (start + visible_capacity).min(candidate_count);
    (start, end)
}

fn candidate_title(candidate: &WindowSwitcherCandidate) -> String {
    let title = candidate.title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }

    "Untitled window".to_owned()
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

fn switcher_grid_layout(
    output_width: u32,
    output_height: u32,
    candidate_count: usize,
) -> Option<SwitcherGridLayout> {
    if candidate_count == 0 {
        return None;
    }

    let max_panel_width = output_width.saturating_sub(SWITCHER_PANEL_SCREEN_MARGIN * 2);
    let max_panel_height = output_height.saturating_sub(SWITCHER_PANEL_SCREEN_MARGIN * 2);
    let content_max_width = max_panel_width.saturating_sub(SWITCHER_PANEL_PADDING * 2);
    let content_max_height = max_panel_height.saturating_sub(
        SWITCHER_PANEL_PADDING * 2 + SWITCHER_HEADER_HEIGHT + SWITCHER_HEADER_GRID_GAP,
    );
    let max_columns =
        grid_axis_capacity(content_max_width, SWITCHER_CARD_WIDTH, SWITCHER_CARD_GAP_X).max(1);
    let max_rows =
        grid_axis_capacity(content_max_height, SWITCHER_CARD_HEIGHT, SWITCHER_CARD_GAP_Y).max(1);
    let columns = candidate_count.min(max_columns).max(1);
    let rows = candidate_count.div_ceil(columns).min(max_rows).max(1);
    let panel_width = SWITCHER_PANEL_PADDING * 2
        + columns as u32 * SWITCHER_CARD_WIDTH
        + columns.saturating_sub(1) as u32 * SWITCHER_CARD_GAP_X;
    let panel_height = SWITCHER_PANEL_PADDING * 2
        + SWITCHER_HEADER_HEIGHT
        + SWITCHER_HEADER_GRID_GAP
        + rows as u32 * SWITCHER_CARD_HEIGHT
        + rows.saturating_sub(1) as u32 * SWITCHER_CARD_GAP_Y;
    let panel_x = (output_width.saturating_sub(panel_width) / 2) as i32;
    let panel_y =
        ((output_height.saturating_sub(panel_height) / 3).max(SWITCHER_PANEL_SCREEN_MARGIN)) as i32;

    Some(SwitcherGridLayout {
        panel_rect: RenderRect { x: panel_x, y: panel_y, width: panel_width, height: panel_height },
        columns,
        visible_capacity: columns.saturating_mul(rows).max(1),
        grid_origin_x: panel_x + SWITCHER_PANEL_PADDING as i32,
        grid_origin_y: panel_y
            + SWITCHER_PANEL_PADDING as i32
            + SWITCHER_HEADER_HEIGHT as i32
            + SWITCHER_HEADER_GRID_GAP as i32,
    })
}

fn grid_axis_capacity(available: u32, item_extent: u32, gap: u32) -> usize {
    ((available.saturating_add(gap)) / item_extent.saturating_add(gap)) as usize
}

fn wrap_switcher_title(title: &str, max_units_per_line: usize) -> Vec<String> {
    let title = title.trim();
    if title.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut remaining = title;
    while !remaining.is_empty() && lines.len() < 2 {
        let (line, rest) = take_title_line(remaining, max_units_per_line);
        if line.is_empty() {
            break;
        }
        lines.push(line);
        remaining = rest.trim_start();
    }

    if !remaining.is_empty() && !lines.is_empty() {
        let last_index = lines.len() - 1;
        lines[last_index] = append_ellipsis(&lines[last_index], max_units_per_line);
    }

    lines
}

fn take_title_line(text: &str, max_units_per_line: usize) -> (String, &str) {
    let mut units = 0;
    let mut last_break = None;
    let mut end = 0;

    for (index, ch) in text.char_indices() {
        let next_units = units + switcher_title_char_units(ch);
        if next_units > max_units_per_line {
            break;
        }
        units = next_units;
        end = index + ch.len_utf8();
        if ch.is_whitespace() || matches!(ch, '-' | '_' | '/' | ':' | '.' | '|') {
            last_break = Some(end);
        }
    }

    if end == 0 {
        let mut chars = text.chars();
        let Some(ch) = chars.next() else {
            return (String::new(), "");
        };
        let end = ch.len_utf8();
        return (text[..end].to_owned(), &text[end..]);
    }

    let split_at = last_break.filter(|break_index| *break_index < end).unwrap_or(end);
    let line = text[..split_at].trim().to_owned();
    let rest = &text[split_at..];
    (line, rest)
}

fn append_ellipsis(line: &str, max_units_per_line: usize) -> String {
    const ELLIPSIS: &str = "...";
    let ellipsis_units = ELLIPSIS.chars().map(switcher_title_char_units).sum::<usize>();
    let mut trimmed = String::new();
    let mut units = 0;

    for ch in line.chars() {
        let next_units = units + switcher_title_char_units(ch);
        if next_units + ellipsis_units > max_units_per_line {
            break;
        }
        units = next_units;
        trimmed.push(ch);
    }

    if trimmed.is_empty() {
        ELLIPSIS.to_owned()
    } else {
        format!("{}{}", trimmed.trim_end(), ELLIPSIS)
    }
}

fn switcher_title_char_units(ch: char) -> usize {
    if ch.is_ascii() { 1 } else { 2 }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bevy_ecs::hierarchy::ChildOf;
    use bevy_ecs::prelude::World;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::LayoutSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        OutputCurrentWorkspace, OutputDevice, OutputId, OutputKind, OutputProperties,
        OutputViewport, Window, WindowLayout, WindowMode, WindowSceneGeometry, WlSurfaceHandle,
        Workspace, WorkspaceId,
    };
    use nekoland_ecs::resources::{
        KeyboardFocusState, OverlayUiFrame, OverlayUiOutputFrame, OverlayUiPrimitive,
        PendingOutputControls, PendingWindowControls, WaylandIngress, WindowStackingState,
        register_entity_index_hooks,
    };
    use nekoland_ecs::selectors::{OutputSelector, SurfaceId};

    use super::{
        WindowSwitcherState, switcher_grid_layout, visible_candidate_page,
        window_switcher_input_system, window_switcher_overlay_system, wrap_switcher_title,
    };

    fn build_switcher_test_app() -> (NekolandApp, OutputId) {
        build_switcher_test_app_with_windows(&[11, 22], 1280, 720)
    }

    fn build_switcher_test_app_with_windows(
        surface_ids: &[u64],
        output_width: u32,
        output_height: u32,
    ) -> (NekolandApp, OutputId) {
        assert!(!surface_ids.is_empty(), "switcher test app needs at least one window");

        let mut app = NekolandApp::new("window-switcher-test");
        register_entity_index_hooks(app.inner_mut().world_mut());
        app.inner_mut()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<nekoland_ecs::resources::PressedKeys>()
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
                    width: output_width,
                    height: output_height,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                viewport: OutputViewport { origin_x: 100, origin_y: 200 },
                ..Default::default()
            },
            OutputCurrentWorkspace { workspace: WorkspaceId(1) },
        ));

        for surface_id in surface_ids {
            let title = format!("window-{surface_id}");
            spawn_window(app.inner_mut().world_mut(), workspace, *surface_id, &title);
        }
        let focused_surface =
            *surface_ids.last().expect("switcher test app needs a focused window");
        app.inner_mut().world_mut().resource_mut::<KeyboardFocusState>().focused_surface =
            Some(focused_surface);
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(output_id);
        app.inner_mut()
            .world_mut()
            .resource_mut::<WindowStackingState>()
            .workspaces
            .insert(1, surface_ids.to_vec());
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
        let mut pressed = world.resource_mut::<nekoland_ecs::resources::PressedKeys>();
        pressed.record_key(64, true);
        pressed.record_key(23, true);
    }

    fn press_alt_shift_tab(world: &mut World) {
        let mut pressed = world.resource_mut::<nekoland_ecs::resources::PressedKeys>();
        pressed.clear_frame_transitions();
        pressed.record_key(23, false);
        pressed.record_key(50, true);
        pressed.record_key(23, true);
    }

    fn release_alt(world: &mut World) {
        let mut pressed = world.resource_mut::<nekoland_ecs::resources::PressedKeys>();
        pressed.clear_frame_transitions();
        pressed.record_key(23, false);
        pressed.record_key(50, false);
        pressed.record_key(64, false);
    }

    fn hold_alt_without_tab(world: &mut World) {
        let mut pressed = world.resource_mut::<nekoland_ecs::resources::PressedKeys>();
        pressed.clear_frame_transitions();
        pressed.record_key(23, false);
    }

    fn overlay_output(app: &NekolandApp, output_id: OutputId) -> &OverlayUiOutputFrame {
        app.inner()
            .world()
            .resource::<OverlayUiFrame>()
            .outputs
            .get(&output_id)
            .expect("switcher overlay should target the origin output")
    }

    fn set_window_title(world: &mut World, surface_id: u64, title: &str) {
        let mut windows = world.query::<(&WlSurfaceHandle, &mut Window)>();
        let Some((_, mut window)) =
            windows.iter_mut(world).find(|(surface, _)| surface.id == surface_id)
        else {
            panic!("window should exist");
        };
        window.title = title.to_owned();
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

        let overlay = overlay_output(&app, output_id);
        assert!(overlay.primitives.iter().any(|primitive| {
            matches!(
                primitive,
                OverlayUiPrimitive::Surface(surface) if surface.surface_id == 11
            )
        }));
    }

    #[test]
    fn switcher_overlay_places_titles_above_thumbnails_without_app_labels() {
        let (mut app, output_id) = build_switcher_test_app();
        press_alt_tab(app.inner_mut().world_mut());

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let overlay = overlay_output(&app, output_id);
        let title_y = overlay
            .primitives
            .iter()
            .find_map(|primitive| match primitive {
                OverlayUiPrimitive::Text(text)
                    if text.id.as_str().starts_with("window_switcher.title.11.") =>
                {
                    Some(text.y)
                }
                _ => None,
            })
            .expect("title primitive should exist for the selected candidate");
        let thumbnail_y = overlay
            .primitives
            .iter()
            .find_map(|primitive| match primitive {
                OverlayUiPrimitive::Surface(surface) if surface.surface_id == 11 => {
                    Some(surface.rect.y)
                }
                _ => None,
            })
            .expect("thumbnail primitive should exist for the selected candidate");

        assert!(title_y < thumbnail_y);
        assert!(!overlay.primitives.iter().any(|primitive| match primitive {
            OverlayUiPrimitive::Text(text) => text.id.as_str().starts_with("window_switcher.app."),
            _ => false,
        }));
    }

    #[test]
    fn switcher_overlay_shows_later_words_from_long_window_titles() {
        let (mut app, output_id) = build_switcher_test_app();
        set_window_title(app.inner_mut().world_mut(), 11, "Window 1 Terminal Project README.md");
        press_alt_tab(app.inner_mut().world_mut());

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let title_lines = overlay_output(&app, output_id)
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                OverlayUiPrimitive::Text(text)
                    if text.id.as_str().starts_with("window_switcher.title.11.") =>
                {
                    Some(text.text.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(title_lines.len(), 2);
        let joined = title_lines.join(" ");
        assert!(joined.contains("README"), "title lines should keep later title words");
        assert!(joined.contains("Terminal"), "title lines should keep the true window title");
    }

    #[test]
    fn switcher_overlay_wraps_cards_into_multiple_rows() {
        let surface_ids = (11_u64..=18).collect::<Vec<_>>();
        let (mut app, output_id) = build_switcher_test_app_with_windows(&surface_ids, 1280, 720);
        press_alt_tab(app.inner_mut().world_mut());

        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let overlay = overlay_output(&app, output_id);
        let positions = overlay
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                OverlayUiPrimitive::Surface(surface) => Some((surface.rect.x, surface.rect.y)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let unique_x = positions.iter().map(|(x, _)| *x).collect::<BTreeSet<_>>();
        let unique_y = positions.iter().map(|(_, y)| *y).collect::<BTreeSet<_>>();

        assert!(unique_x.len() > 1, "wrapped grid should use multiple columns");
        assert!(unique_y.len() > 1, "wrapped grid should use multiple rows");
    }

    #[test]
    fn switcher_overlay_pages_visible_cards_around_the_selected_window() {
        let surface_ids = (11_u64..=40).collect::<Vec<_>>();
        let (mut app, output_id) = build_switcher_test_app_with_windows(&surface_ids, 1280, 720);
        press_alt_tab(app.inner_mut().world_mut());
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        hold_alt_without_tab(app.inner_mut().world_mut());
        let selected_index = 25;
        let candidate_order = surface_ids.iter().copied().rev().collect::<Vec<_>>();
        let selected_surface = candidate_order[selected_index];
        {
            let mut switcher = app.inner_mut().world_mut().resource_mut::<WindowSwitcherState>();
            switcher.selected_index = selected_index;
            switcher.selected_surface = Some(SurfaceId(selected_surface));
        }
        app.inner_mut().world_mut().resource_mut::<OverlayUiFrame>().clear();
        app.inner_mut().world_mut().run_schedule(LayoutSchedule);

        let layout = switcher_grid_layout(1280, 720, candidate_order.len())
            .expect("grid layout should exist");
        let (start, end) =
            visible_candidate_page(candidate_order.len(), selected_index, layout.visible_capacity);
        let expected_visible = candidate_order[start..end].iter().copied().collect::<BTreeSet<_>>();
        let actual_visible = overlay_output(&app, output_id)
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                OverlayUiPrimitive::Surface(surface) => Some(surface.surface_id),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(actual_visible, expected_visible);
        assert!(actual_visible.contains(&selected_surface));
        assert_eq!(actual_visible.len(), end - start);
    }

    #[test]
    fn wrap_switcher_title_uses_two_lines_before_truncating() {
        let lines = wrap_switcher_title("Window 1 Terminal Project README.md", 18);

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("Window"));
        assert!(lines[1].contains("Project") || lines[1].contains("README"));
    }
}
