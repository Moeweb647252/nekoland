//! Workspace-local column/row tiling state and geometry helpers.

#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::SurfaceGeometry;

use super::WorkArea;

/// Synthetic workspace bucket used before a surface belongs to a concrete workspace.
pub const UNASSIGNED_WORKSPACE_TILING_ID: u32 = 0;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HorizontalDirection {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VerticalDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TilingPanDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TilingCoordinates {
    pub column_index: usize,
    pub row_index: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TiledColumn {
    pub surface_ids: Vec<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceColumnLayout {
    pub columns: Vec<TiledColumn>,
}

impl WorkspaceColumnLayout {
    pub fn ensure_surface(&mut self, surface_id: u64) {
        if self.coordinates_for_surface(surface_id).is_some() {
            return;
        }

        self.columns.push(TiledColumn { surface_ids: vec![surface_id] });
    }

    pub fn retain_surfaces(&mut self, known_surfaces: &BTreeSet<u64>) {
        for column in &mut self.columns {
            column.surface_ids.retain(|surface_id| known_surfaces.contains(surface_id));
        }
        self.columns.retain(|column| !column.surface_ids.is_empty());
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub fn coordinates_for_surface(&self, surface_id: u64) -> Option<TilingCoordinates> {
        self.columns.iter().enumerate().find_map(|(column_index, column)| {
            column
                .surface_ids
                .iter()
                .position(|candidate| *candidate == surface_id)
                .map(|row_index| TilingCoordinates { column_index, row_index })
        })
    }

    pub fn arranged_geometry(&self, work_area: &WorkArea) -> BTreeMap<u64, SurfaceGeometry> {
        let mut geometry = BTreeMap::new();
        for (column_index, column) in self.columns.iter().enumerate() {
            let column_x = column_origin_x(work_area, column_index);
            let row_count = column.surface_ids.len().max(1);
            for (row_index, surface_id) in column.surface_ids.iter().copied().enumerate() {
                let (row_y, row_height) = split_extent(work_area.y as isize, work_area.height, row_count, row_index);
                geometry.insert(
                    surface_id,
                    SurfaceGeometry {
                        x: saturating_isize_to_i32(column_x),
                        y: saturating_isize_to_i32(row_y),
                        width: work_area.width.max(1),
                        height: row_height.max(1),
                    },
                );
            }
        }
        geometry
    }

    pub fn column_origins(&self, work_area: &WorkArea) -> Vec<isize> {
        (0..self.columns.len()).map(|index| column_origin_x(work_area, index)).collect()
    }

    pub fn row_origins(&self, work_area: &WorkArea, column_index: usize) -> Vec<isize> {
        let Some(column) = self.columns.get(column_index) else {
            return Vec::new();
        };
        (0..column.surface_ids.len())
            .map(|row_index| split_extent(work_area.y as isize, work_area.height, column.surface_ids.len(), row_index).0)
            .collect()
    }

    pub fn focus_column(&self, surface_id: u64, direction: HorizontalDirection) -> Option<u64> {
        let coords = self.coordinates_for_surface(surface_id)?;
        let target_column_index = match direction {
            HorizontalDirection::Left => coords.column_index.checked_sub(1)?,
            HorizontalDirection::Right => {
                let next = coords.column_index.saturating_add(1);
                (next < self.columns.len()).then_some(next)?
            }
        };
        let target_column = self.columns.get(target_column_index)?;
        let target_row_index = coords.row_index.min(target_column.surface_ids.len().saturating_sub(1));
        target_column.surface_ids.get(target_row_index).copied()
    }

    pub fn focus_window(&self, surface_id: u64, direction: VerticalDirection) -> Option<u64> {
        let coords = self.coordinates_for_surface(surface_id)?;
        let column = self.columns.get(coords.column_index)?;
        let target_row_index = match direction {
            VerticalDirection::Up => coords.row_index.checked_sub(1)?,
            VerticalDirection::Down => {
                let next = coords.row_index.saturating_add(1);
                (next < column.surface_ids.len()).then_some(next)?
            }
        };
        column.surface_ids.get(target_row_index).copied()
    }

    pub fn move_column(&mut self, surface_id: u64, direction: HorizontalDirection) -> bool {
        let Some(coords) = self.coordinates_for_surface(surface_id) else {
            return false;
        };
        let swap_index = match direction {
            HorizontalDirection::Left => match coords.column_index.checked_sub(1) {
                Some(index) => index,
                None => return false,
            },
            HorizontalDirection::Right => {
                let next = coords.column_index.saturating_add(1);
                if next >= self.columns.len() {
                    return false;
                }
                next
            }
        };
        self.columns.swap(coords.column_index, swap_index);
        true
    }

    pub fn move_window(&mut self, surface_id: u64, direction: VerticalDirection) -> bool {
        let Some(coords) = self.coordinates_for_surface(surface_id) else {
            return false;
        };
        let Some(column) = self.columns.get_mut(coords.column_index) else {
            return false;
        };
        let swap_index = match direction {
            VerticalDirection::Up => match coords.row_index.checked_sub(1) {
                Some(index) => index,
                None => return false,
            },
            VerticalDirection::Down => {
                let next = coords.row_index.saturating_add(1);
                if next >= column.surface_ids.len() {
                    return false;
                }
                next
            }
        };
        column.surface_ids.swap(coords.row_index, swap_index);
        true
    }

    pub fn consume_into_column(
        &mut self,
        surface_id: u64,
        direction: HorizontalDirection,
    ) -> bool {
        let Some(coords) = self.coordinates_for_surface(surface_id) else {
            return false;
        };
        let target_column_index = match direction {
            HorizontalDirection::Left => match coords.column_index.checked_sub(1) {
                Some(index) => index,
                None => return false,
            },
            HorizontalDirection::Right => {
                let next = coords.column_index.saturating_add(1);
                if next >= self.columns.len() {
                    return false;
                }
                next
            }
        };
        if coords.column_index == target_column_index {
            return false;
        }

        let surface_id = match self.columns.get_mut(coords.column_index) {
            Some(column) if coords.row_index < column.surface_ids.len() => {
                column.surface_ids.remove(coords.row_index)
            }
            _ => return false,
        };
        let source_column_was_emptied = self
            .columns
            .get(coords.column_index)
            .is_some_and(|column| column.surface_ids.is_empty());
        let adjusted_target_index = match direction {
            HorizontalDirection::Left => target_column_index,
            HorizontalDirection::Right if source_column_was_emptied => target_column_index.saturating_sub(1),
            HorizontalDirection::Right => target_column_index,
        };

        if source_column_was_emptied {
            self.columns.remove(coords.column_index);
        }

        let Some(target_column) = self.columns.get_mut(adjusted_target_index) else {
            return false;
        };
        target_column.surface_ids.push(surface_id);
        true
    }

    pub fn expel_from_column(
        &mut self,
        surface_id: u64,
        direction: HorizontalDirection,
    ) -> bool {
        let Some(coords) = self.coordinates_for_surface(surface_id) else {
            return false;
        };
        let Some(source_column) = self.columns.get(coords.column_index) else {
            return false;
        };
        if source_column.surface_ids.len() <= 1 {
            return false;
        }

        let surface_id = self.columns[coords.column_index].surface_ids.remove(coords.row_index);
        let insert_index = match direction {
            HorizontalDirection::Left => coords.column_index,
            HorizontalDirection::Right => coords.column_index.saturating_add(1),
        };
        self.columns.insert(insert_index, TiledColumn { surface_ids: vec![surface_id] });
        true
    }

    pub fn snapped_viewport_for_surface(
        &self,
        work_area: &WorkArea,
        surface_id: u64,
    ) -> Option<(isize, isize)> {
        let coords = self.coordinates_for_surface(surface_id)?;
        Some((
            column_origin_x(work_area, coords.column_index),
            self.row_origins(work_area, coords.column_index).get(coords.row_index).copied()?,
        ))
    }

    pub fn snapped_viewport_after_pan(
        &self,
        work_area: &WorkArea,
        focused_surface: Option<u64>,
        current_origin_x: isize,
        current_origin_y: isize,
        direction: TilingPanDirection,
    ) -> Option<(isize, isize)> {
        match direction {
            TilingPanDirection::Left | TilingPanDirection::Right => {
                let origins = self.column_origins(work_area);
                let current_index = nearest_anchor_index(&origins, current_origin_x)?;
                let target_index = match direction {
                    TilingPanDirection::Left => current_index.checked_sub(1)?,
                    TilingPanDirection::Right => {
                        let next = current_index.saturating_add(1);
                        (next < origins.len()).then_some(next)?
                    }
                    TilingPanDirection::Up | TilingPanDirection::Down => unreachable!(),
                };
                Some((origins[target_index], current_origin_y))
            }
            TilingPanDirection::Up | TilingPanDirection::Down => {
                let focused_surface = focused_surface?;
                let coords = self.coordinates_for_surface(focused_surface)?;
                let origins = self.row_origins(work_area, coords.column_index);
                let current_index = nearest_anchor_index(&origins, current_origin_y)?;
                let target_index = match direction {
                    TilingPanDirection::Up => current_index.checked_sub(1)?,
                    TilingPanDirection::Down => {
                        let next = current_index.saturating_add(1);
                        (next < origins.len()).then_some(next)?
                    }
                    TilingPanDirection::Left | TilingPanDirection::Right => unreachable!(),
                };
                Some((current_origin_x, origins[target_index]))
            }
        }
    }
}

/// Workspace-scoped tiling state used by the shell layout systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTilingState {
    pub workspaces: BTreeMap<u32, WorkspaceColumnLayout>,
}

impl WorkspaceTilingState {
    pub fn ensure_surface(&mut self, workspace_id: u32, surface_id: u64) {
        self.workspaces.entry(workspace_id).or_default().ensure_surface(surface_id);
    }

    pub fn retain_known(&mut self, known_surfaces: &BTreeMap<u64, u32>) {
        let mut known_by_workspace = BTreeMap::<u32, BTreeSet<u64>>::new();
        for (surface_id, workspace_id) in known_surfaces {
            known_by_workspace.entry(*workspace_id).or_default().insert(*surface_id);
        }

        let empty = BTreeSet::new();
        let mut empty_workspaces = Vec::new();
        for (workspace_id, layout) in &mut self.workspaces {
            layout.retain_surfaces(known_by_workspace.get(workspace_id).unwrap_or(&empty));
            if layout.is_empty() {
                empty_workspaces.push(*workspace_id);
            }
        }

        for workspace_id in empty_workspaces {
            self.workspaces.remove(&workspace_id);
        }
    }

    pub fn arranged_geometry(&self, work_area: &WorkArea) -> BTreeMap<u64, SurfaceGeometry> {
        let mut geometry = BTreeMap::new();
        for layout in self.workspaces.values() {
            geometry.extend(layout.arranged_geometry(work_area));
        }
        geometry
    }

    pub fn coordinates_for_surface(
        &self,
        workspace_id: u32,
        surface_id: u64,
    ) -> Option<TilingCoordinates> {
        self.workspaces.get(&workspace_id)?.coordinates_for_surface(surface_id)
    }

    pub fn focus_column(
        &self,
        workspace_id: u32,
        surface_id: u64,
        direction: HorizontalDirection,
    ) -> Option<u64> {
        self.workspaces.get(&workspace_id)?.focus_column(surface_id, direction)
    }

    pub fn focus_window(
        &self,
        workspace_id: u32,
        surface_id: u64,
        direction: VerticalDirection,
    ) -> Option<u64> {
        self.workspaces.get(&workspace_id)?.focus_window(surface_id, direction)
    }

    pub fn move_column(
        &mut self,
        workspace_id: u32,
        surface_id: u64,
        direction: HorizontalDirection,
    ) -> bool {
        self.workspaces
            .entry(workspace_id)
            .or_default()
            .move_column(surface_id, direction)
    }

    pub fn move_window(
        &mut self,
        workspace_id: u32,
        surface_id: u64,
        direction: VerticalDirection,
    ) -> bool {
        self.workspaces
            .entry(workspace_id)
            .or_default()
            .move_window(surface_id, direction)
    }

    pub fn consume_into_column(
        &mut self,
        workspace_id: u32,
        surface_id: u64,
        direction: HorizontalDirection,
    ) -> bool {
        self.workspaces
            .entry(workspace_id)
            .or_default()
            .consume_into_column(surface_id, direction)
    }

    pub fn expel_from_column(
        &mut self,
        workspace_id: u32,
        surface_id: u64,
        direction: HorizontalDirection,
    ) -> bool {
        self.workspaces
            .entry(workspace_id)
            .or_default()
            .expel_from_column(surface_id, direction)
    }

    pub fn snapped_viewport_for_surface(
        &self,
        workspace_id: u32,
        work_area: &WorkArea,
        surface_id: u64,
    ) -> Option<(isize, isize)> {
        self.workspaces.get(&workspace_id)?.snapped_viewport_for_surface(work_area, surface_id)
    }

    pub fn snapped_viewport_after_pan(
        &self,
        workspace_id: u32,
        work_area: &WorkArea,
        focused_surface: Option<u64>,
        current_origin_x: isize,
        current_origin_y: isize,
        direction: TilingPanDirection,
    ) -> Option<(isize, isize)> {
        self.workspaces.get(&workspace_id)?.snapped_viewport_after_pan(
            work_area,
            focused_surface,
            current_origin_x,
            current_origin_y,
            direction,
        )
    }
}

fn column_origin_x(work_area: &WorkArea, column_index: usize) -> isize {
    let step = work_area.width.max(1) as isize;
    (work_area.x as isize).saturating_add(step.saturating_mul(column_index as isize))
}

fn split_extent(origin: isize, total: u32, count: usize, index: usize) -> (isize, u32) {
    let count = count.max(1) as u32;
    let index = index.min(count.saturating_sub(1) as usize) as u32;
    let base = total / count;
    let remainder = total % count;
    let size = base.saturating_add(u32::from(index < remainder)).max(1);
    let offset = index
        .saturating_mul(base)
        .saturating_add(remainder.min(index)) as isize;
    (origin.saturating_add(offset), size)
}

fn nearest_anchor_index(anchors: &[isize], current_origin: isize) -> Option<usize> {
    anchors.iter().enumerate().min_by_key(|(_, anchor)| anchor.abs_diff(current_origin)).map(
        |(index, _)| index,
    )
}

fn saturating_isize_to_i32(value: isize) -> i32 {
    value.clamp(i32::MIN as isize, i32::MAX as isize) as i32
}

#[cfg(test)]
mod tests {
    use super::{
        HorizontalDirection, TilingCoordinates, TilingPanDirection,
        UNASSIGNED_WORKSPACE_TILING_ID, VerticalDirection, WorkspaceColumnLayout,
        WorkspaceTilingState,
    };
    use crate::resources::WorkArea;

    #[test]
    fn ensure_surface_appends_new_columns_in_discovery_order() {
        let mut layout = WorkspaceColumnLayout::default();
        layout.ensure_surface(11);
        layout.ensure_surface(22);
        layout.ensure_surface(33);

        assert_eq!(layout.columns.len(), 3);
        assert_eq!(layout.columns[0].surface_ids, vec![11]);
        assert_eq!(layout.columns[1].surface_ids, vec![22]);
        assert_eq!(layout.columns[2].surface_ids, vec![33]);
    }

    #[test]
    fn arranged_geometry_uses_full_width_columns_and_equal_rows() {
        let mut layout = WorkspaceColumnLayout::default();
        layout.ensure_surface(11);
        layout.ensure_surface(22);
        assert!(layout.consume_into_column(22, HorizontalDirection::Left));
        layout.ensure_surface(33);

        let geometry = layout.arranged_geometry(&WorkArea { x: 10, y: 20, width: 1000, height: 900 });
        assert_eq!(geometry[&11].x, 10);
        assert_eq!(geometry[&11].y, 20);
        assert_eq!(geometry[&11].width, 1000);
        assert_eq!(geometry[&11].height, 450);
        assert_eq!(geometry[&22].x, 10);
        assert_eq!(geometry[&22].y, 470);
        assert_eq!(geometry[&22].height, 450);
        assert_eq!(geometry[&33].x, 1010);
        assert_eq!(geometry[&33].width, 1000);
    }

    #[test]
    fn focus_and_move_operations_follow_column_row_structure() {
        let mut layout = WorkspaceColumnLayout::default();
        layout.ensure_surface(11);
        layout.ensure_surface(22);
        layout.ensure_surface(33);
        assert!(layout.consume_into_column(33, HorizontalDirection::Left));

        assert_eq!(layout.coordinates_for_surface(22), Some(TilingCoordinates { column_index: 1, row_index: 0 }));
        assert_eq!(layout.focus_column(22, HorizontalDirection::Left), Some(11));
        assert_eq!(layout.focus_window(22, VerticalDirection::Down), Some(33));
        assert!(layout.move_window(33, VerticalDirection::Up));
        assert_eq!(layout.columns[1].surface_ids, vec![33, 22]);
        assert!(layout.move_column(22, HorizontalDirection::Left));
        assert_eq!(layout.columns[0].surface_ids, vec![33, 22]);
    }

    #[test]
    fn expel_and_retain_prune_empty_columns() {
        let mut tiling = WorkspaceTilingState::default();
        tiling.ensure_surface(UNASSIGNED_WORKSPACE_TILING_ID, 11);
        tiling.ensure_surface(UNASSIGNED_WORKSPACE_TILING_ID, 22);
        assert!(tiling.consume_into_column(UNASSIGNED_WORKSPACE_TILING_ID, 22, HorizontalDirection::Left));
        assert!(tiling.expel_from_column(UNASSIGNED_WORKSPACE_TILING_ID, 22, HorizontalDirection::Right));

        let layout = tiling.workspaces.get(&UNASSIGNED_WORKSPACE_TILING_ID).expect("workspace");
        assert_eq!(layout.columns.len(), 2);
        assert_eq!(layout.columns[0].surface_ids, vec![11]);
        assert_eq!(layout.columns[1].surface_ids, vec![22]);

        tiling.retain_known(&[(22, 3_u32)].into_iter().collect());
        tiling.ensure_surface(3, 22);
        assert!(!tiling.workspaces.contains_key(&UNASSIGNED_WORKSPACE_TILING_ID));
        assert_eq!(tiling.workspaces[&3].columns.len(), 1);
    }

    #[test]
    fn snapped_viewport_helpers_use_column_and_row_anchors() {
        let mut layout = WorkspaceColumnLayout::default();
        layout.ensure_surface(11);
        layout.ensure_surface(22);
        assert!(layout.consume_into_column(22, HorizontalDirection::Left));
        layout.ensure_surface(33);
        let work_area = WorkArea { x: 0, y: 32, width: 1280, height: 688 };

        assert_eq!(layout.snapped_viewport_for_surface(&work_area, 33), Some((1280, 32)));
        assert_eq!(
            layout.snapped_viewport_after_pan(&work_area, Some(11), 75, 500, TilingPanDirection::Right),
            Some((1280, 500))
        );
        assert_eq!(
            layout.snapped_viewport_after_pan(&work_area, Some(11), 0, 500, TilingPanDirection::Up),
            Some((0, 32))
        );
    }
}
