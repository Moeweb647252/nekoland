use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::SurfaceGeometry;

use super::WorkArea;

pub const UNASSIGNED_WORKSPACE_TILING_ID: u32 = 0;

/// Split axis for one internal tiling-tree node.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SplitAxis {
    #[default]
    Horizontal,
    Vertical,
}

impl SplitAxis {
    pub const fn alternate(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }
}

/// Stable identifier for one node inside a workspace tile tree.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct TileNodeId(pub u64);

/// One node in the binary tiling tree.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TileNode {
    Leaf { surface_id: u64 },
    Split { axis: SplitAxis, first: TileNodeId, second: TileNodeId },
}

/// Workspace-local tiling tree plus stable leaf order.
///
/// The current implementation keeps insertion order stable and rebuilds a simple master/stack tree
/// whenever the leaf set changes. That gives us a real tree-shaped runtime model now while still
/// leaving room for explicit split manipulation later.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTileTree {
    pub root: Option<TileNodeId>,
    pub nodes: BTreeMap<TileNodeId, TileNode>,
    pub parents: BTreeMap<TileNodeId, TileNodeId>,
    pub surface_nodes: BTreeMap<u64, TileNodeId>,
    pub root_axis: SplitAxis,
    pub leaf_surfaces: Vec<u64>,
    pub next_node_id: u64,
}

impl WorkspaceTileTree {
    pub fn ensure_surface(&mut self, surface_id: u64) {
        if self.surface_nodes.contains_key(&surface_id) {
            return;
        }

        self.leaf_surfaces.push(surface_id);
        self.insert_surface_node(surface_id);
    }

    pub fn retain_surfaces(&mut self, known_surfaces: &BTreeSet<u64>) {
        let removed = self
            .leaf_surfaces
            .iter()
            .copied()
            .filter(|surface_id| !known_surfaces.contains(surface_id))
            .collect::<Vec<_>>();
        for surface_id in removed {
            self.remove_surface(surface_id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.leaf_surfaces.is_empty()
    }

    pub fn set_root_axis(&mut self, axis: SplitAxis) {
        self.root_axis = axis;
        if let Some(root) = self.root
            && let Some(TileNode::Split { axis: split_axis, .. }) = self.nodes.get_mut(&root)
        {
            *split_axis = axis;
        }
    }

    pub fn set_surface_split_axis(&mut self, surface_id: u64, axis: SplitAxis) {
        let Some(node_id) = self.surface_nodes.get(&surface_id).copied() else {
            return;
        };
        let Some(parent_id) = self.parents.get(&node_id).copied() else {
            self.set_root_axis(axis);
            return;
        };
        if let Some(TileNode::Split { axis: split_axis, .. }) = self.nodes.get_mut(&parent_id) {
            *split_axis = axis;
        }
    }

    pub fn arranged_geometry(&self, work_area: &WorkArea) -> BTreeMap<u64, SurfaceGeometry> {
        let Some(root) = self.root else {
            return BTreeMap::new();
        };

        let mut geometry = BTreeMap::new();
        self.collect_geometry(
            root,
            TileRect {
                x: work_area.x,
                y: work_area.y,
                width: work_area.width.max(1),
                height: work_area.height.max(1),
            },
            &mut geometry,
        );
        geometry
    }

    pub fn split_axis_for_surface(&self, surface_id: u64) -> Option<SplitAxis> {
        let node_id = self.surface_nodes.get(&surface_id).copied()?;
        let parent_id = self.parents.get(&node_id).copied()?;
        match self.nodes.get(&parent_id) {
            Some(TileNode::Split { axis, .. }) => Some(*axis),
            _ => None,
        }
    }

    fn insert_surface_node(&mut self, surface_id: u64) {
        let new_leaf = self.alloc_node();
        self.nodes.insert(new_leaf, TileNode::Leaf { surface_id });
        self.surface_nodes.insert(surface_id, new_leaf);

        let Some(existing_tail_surface) = self.leaf_surfaces.iter().rev().nth(1).copied() else {
            self.root = Some(new_leaf);
            return;
        };
        let tail_leaf = self
            .surface_nodes
            .get(&existing_tail_surface)
            .copied()
            .expect("existing tail leaf should be indexed");
        let parent = self.parents.get(&tail_leaf).copied();
        let axis = parent
            .and_then(|parent_id| match self.nodes.get(&parent_id) {
                Some(TileNode::Split { axis, .. }) => Some(axis.alternate()),
                _ => None,
            })
            .unwrap_or(self.root_axis);
        let split = self.alloc_node();
        self.nodes.insert(split, TileNode::Split { axis, first: tail_leaf, second: new_leaf });
        self.parents.insert(tail_leaf, split);
        self.parents.insert(new_leaf, split);

        match parent {
            Some(parent_id) => {
                self.parents.insert(split, parent_id);
                match self.nodes.get_mut(&parent_id) {
                    Some(TileNode::Split { first, second, .. }) if *first == tail_leaf => {
                        *first = split;
                    }
                    Some(TileNode::Split { first: _, second, .. }) if *second == tail_leaf => {
                        *second = split;
                    }
                    _ => {}
                }
            }
            None => self.root = Some(split),
        }
    }

    fn remove_surface(&mut self, surface_id: u64) {
        let Some(node_id) = self.surface_nodes.remove(&surface_id) else {
            return;
        };
        self.leaf_surfaces.retain(|current| *current != surface_id);
        let parent = self.parents.remove(&node_id);

        match parent {
            None => {
                self.nodes.remove(&node_id);
                self.root = None;
            }
            Some(parent_id) => {
                let (first, second) = match self.nodes.get(&parent_id) {
                    Some(TileNode::Split { first, second, .. }) => (*first, *second),
                    _ => return,
                };
                let sibling = if first == node_id { second } else { first };
                let grandparent = self.parents.remove(&parent_id);

                if let Some(grandparent_id) = grandparent {
                    match self.nodes.get_mut(&grandparent_id) {
                        Some(TileNode::Split { first, second, .. }) if *first == parent_id => {
                            *first = sibling;
                        }
                        Some(TileNode::Split { first: _, second, .. }) if *second == parent_id => {
                            *second = sibling;
                        }
                        _ => {}
                    }
                    self.parents.insert(sibling, grandparent_id);
                } else {
                    self.root = Some(sibling);
                    self.parents.remove(&sibling);
                }

                self.nodes.remove(&node_id);
                self.nodes.remove(&parent_id);
            }
        }
    }

    fn alloc_node(&mut self) -> TileNodeId {
        let next = self.next_node_id.max(1);
        self.next_node_id = next.saturating_add(1);
        TileNodeId(next)
    }

    pub fn rebuild_from_leaf_order(&mut self) {
        let leaf_surfaces = self.leaf_surfaces.clone();
        let root_axis = self.root_axis;
        *self = Self { root_axis, ..Self::default() };
        for surface_id in leaf_surfaces {
            self.ensure_surface(surface_id);
        }
    }

    fn collect_geometry(
        &self,
        node_id: TileNodeId,
        rect: TileRect,
        geometry: &mut BTreeMap<u64, SurfaceGeometry>,
    ) {
        let Some(node) = self.nodes.get(&node_id) else {
            return;
        };

        match node {
            TileNode::Leaf { surface_id } => {
                geometry.insert(*surface_id, rect.into());
            }
            TileNode::Split { axis, first, second } => {
                let (first_rect, second_rect) = rect.split(*axis);
                self.collect_geometry(*first, first_rect, geometry);
                self.collect_geometry(*second, second_rect, geometry);
            }
        }
    }
}

/// Workspace-scoped tiling state used by the shell layout systems.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceTilingState {
    pub workspaces: BTreeMap<u32, WorkspaceTileTree>,
}

impl WorkspaceTilingState {
    pub fn ensure_surface(&mut self, workspace_id: u32, surface_id: u64) {
        self.workspaces.entry(workspace_id).or_default().ensure_surface(surface_id);
    }

    pub fn set_root_axis(&mut self, workspace_id: u32, axis: SplitAxis) {
        self.workspaces.entry(workspace_id).or_default().set_root_axis(axis);
    }

    pub fn set_surface_split_axis(&mut self, workspace_id: u32, surface_id: u64, axis: SplitAxis) {
        self.workspaces.entry(workspace_id).or_default().set_surface_split_axis(surface_id, axis);
    }

    pub fn retain_known(&mut self, known_surfaces: &BTreeMap<u64, u32>) {
        let mut known_by_workspace = BTreeMap::<u32, BTreeSet<u64>>::new();
        for (surface_id, workspace_id) in known_surfaces {
            known_by_workspace.entry(*workspace_id).or_default().insert(*surface_id);
        }

        let empty = BTreeSet::new();
        let mut empty_workspaces = Vec::new();
        for (workspace_id, tree) in &mut self.workspaces {
            tree.retain_surfaces(known_by_workspace.get(workspace_id).unwrap_or(&empty));
            if tree.is_empty() {
                empty_workspaces.push(*workspace_id);
            }
        }

        for workspace_id in empty_workspaces {
            self.workspaces.remove(&workspace_id);
        }
    }

    pub fn arranged_geometry(&self, work_area: &WorkArea) -> BTreeMap<u64, SurfaceGeometry> {
        let mut geometry = BTreeMap::new();
        for tree in self.workspaces.values() {
            geometry.extend(tree.arranged_geometry(work_area));
        }
        geometry
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TileRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl TileRect {
    fn split(self, axis: SplitAxis) -> (Self, Self) {
        match axis {
            SplitAxis::Horizontal => self.split_horizontal(),
            SplitAxis::Vertical => self.split_vertical(),
        }
    }

    fn split_horizontal(self) -> (Self, Self) {
        if self.width <= 1 {
            return (self, self);
        }

        let first_width = self.width / 2;
        let second_width = self.width.saturating_sub(first_width).max(1);
        let first_width = self.width.saturating_sub(second_width);

        (
            Self { width: first_width.max(1), ..self },
            Self { x: self.x + first_width as i32, width: second_width, ..self },
        )
    }

    fn split_vertical(self) -> (Self, Self) {
        if self.height <= 1 {
            return (self, self);
        }

        let first_height = self.height / 2;
        let second_height = self.height.saturating_sub(first_height).max(1);
        let first_height = self.height.saturating_sub(second_height);

        (
            Self { height: first_height.max(1), ..self },
            Self { y: self.y + first_height as i32, height: second_height, ..self },
        )
    }
}

impl From<TileRect> for SurfaceGeometry {
    fn from(value: TileRect) -> Self {
        Self { x: value.x, y: value.y, width: value.width.max(1), height: value.height.max(1) }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SplitAxis, TileNode, UNASSIGNED_WORKSPACE_TILING_ID, WorkspaceTileTree,
        WorkspaceTilingState,
    };
    use crate::resources::WorkArea;

    #[test]
    fn workspace_tile_tree_builds_master_stack_root_from_leaf_order() {
        let mut tree = WorkspaceTileTree::default();
        tree.ensure_surface(11);
        tree.ensure_surface(22);
        tree.ensure_surface(33);

        assert_eq!(tree.leaf_surfaces, vec![11, 22, 33]);
        let root = tree.root.expect("tree root should exist");
        let node = tree.nodes.get(&root).expect("root node should exist");
        assert!(matches!(node, TileNode::Split { axis: SplitAxis::Horizontal, .. }));
    }

    #[test]
    fn surface_split_axis_updates_parent_split_without_rebuilding_tree() {
        let mut tree = WorkspaceTileTree::default();
        tree.ensure_surface(11);
        tree.ensure_surface(22);
        tree.ensure_surface(33);

        tree.set_surface_split_axis(22, SplitAxis::Horizontal);

        assert_eq!(tree.split_axis_for_surface(22), Some(SplitAxis::Horizontal));
        assert_eq!(tree.split_axis_for_surface(33), Some(SplitAxis::Horizontal));
        assert_eq!(tree.split_axis_for_surface(11), Some(SplitAxis::Horizontal));
    }

    #[test]
    fn arranged_geometry_splits_work_area_across_all_leaves() {
        let mut tree = WorkspaceTileTree::default();
        tree.ensure_surface(11);
        tree.ensure_surface(22);
        tree.ensure_surface(33);

        let geometry = tree.arranged_geometry(&WorkArea { x: 0, y: 0, width: 1200, height: 900 });
        assert_eq!(geometry.len(), 3);
        assert_eq!(geometry[&11].x, 0);
        assert_eq!(geometry[&11].width, 600);
        assert_eq!(geometry[&22].x, 600);
        assert_eq!(geometry[&22].height, 450);
        assert_eq!(geometry[&33].x, 600);
        assert_eq!(geometry[&33].y, 450);
    }

    #[test]
    fn workspace_tiling_state_moves_surfaces_between_workspaces() {
        let mut tiling = WorkspaceTilingState::default();
        tiling.ensure_surface(UNASSIGNED_WORKSPACE_TILING_ID, 11);
        tiling.ensure_surface(2, 22);
        tiling.retain_known(&[(11, 3_u32), (22, 2_u32)].into_iter().collect());
        tiling.ensure_surface(3, 11);

        assert!(!tiling.workspaces.contains_key(&UNASSIGNED_WORKSPACE_TILING_ID));
        assert_eq!(tiling.workspaces.get(&2).expect("workspace 2").leaf_surfaces, vec![22]);
        assert_eq!(tiling.workspaces.get(&3).expect("workspace 3").leaf_surfaces, vec![11]);
    }

    #[test]
    fn retained_tree_keeps_explicit_split_axes_for_remaining_surfaces() {
        let mut tiling = WorkspaceTilingState::default();
        tiling.ensure_surface(1, 11);
        tiling.ensure_surface(1, 22);
        tiling.ensure_surface(1, 33);
        tiling.set_surface_split_axis(1, 22, SplitAxis::Horizontal);
        tiling.retain_known(&[(11, 1_u32), (22, 1_u32)].into_iter().collect());

        let tree = tiling.workspaces.get(&1).expect("workspace tree should remain");
        assert_eq!(tree.leaf_surfaces, vec![11, 22]);
        assert_eq!(tree.split_axis_for_surface(11), Some(SplitAxis::Horizontal));
        assert_eq!(tree.split_axis_for_surface(22), Some(SplitAxis::Horizontal));
    }
}
