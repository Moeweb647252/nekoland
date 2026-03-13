use std::collections::BTreeMap;
use std::collections::BTreeSet;

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

pub const UNASSIGNED_WORKSPACE_STACK_ID: u32 = 0;

/// Back-to-front z-order of managed windows, tracked independently per workspace.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowStackingState {
    pub workspaces: BTreeMap<u32, Vec<u64>>,
}

impl WindowStackingState {
    /// Ensures a surface participates in the stack without disturbing the current order.
    pub fn ensure(&mut self, workspace_id: u32, surface_id: u64) {
        let stack = self.workspaces.entry(workspace_id).or_default();
        if !stack.contains(&surface_id) {
            stack.push(surface_id);
        }
    }

    /// Moves a surface to the front-most position, inserting it when first seen.
    pub fn raise(&mut self, workspace_id: u32, surface_id: u64) {
        let stack = self.workspaces.entry(workspace_id).or_default();
        stack.retain(|existing| *existing != surface_id);
        stack.push(surface_id);
    }

    /// Drops surfaces that are no longer known to the shell and keeps surfaces only in the
    /// workspace bucket they currently belong to.
    pub fn retain_known(&mut self, known_surfaces: &BTreeMap<u64, u32>) {
        self.workspaces.retain(|workspace_id, stack| {
            stack.retain(|surface_id| known_surfaces.get(surface_id) == Some(workspace_id));
            !stack.is_empty()
        });
    }

    /// Orders an arbitrary surface list according to the current stack and appends unknown ids in
    /// their original relative order within each workspace.
    pub fn ordered_surfaces<I>(&self, surface_ids: I) -> Vec<u64>
    where
        I: IntoIterator<Item = (u32, u64)>,
    {
        let mut grouped = BTreeMap::<u32, Vec<u64>>::new();
        for (workspace_id, surface_id) in surface_ids {
            grouped.entry(workspace_id).or_default().push(surface_id);
        }

        let mut ordered = Vec::new();
        for (workspace_id, surfaces) in grouped {
            let mut remaining = surfaces.iter().copied().collect::<BTreeSet<_>>();
            if let Some(stack) = self.workspaces.get(&workspace_id) {
                ordered.extend(
                    stack.iter().copied().filter(|surface_id| remaining.remove(surface_id)),
                );
            }
            ordered.extend(surfaces.into_iter().filter(|surface_id| remaining.remove(surface_id)));
        }
        ordered
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{UNASSIGNED_WORKSPACE_STACK_ID, WindowStackingState};

    #[test]
    fn raise_moves_surface_to_front() {
        let mut stacking = WindowStackingState {
            workspaces: BTreeMap::from([(UNASSIGNED_WORKSPACE_STACK_ID, vec![11, 22, 33])]),
        };

        stacking.raise(UNASSIGNED_WORKSPACE_STACK_ID, 22);

        assert_eq!(
            stacking.workspaces.get(&UNASSIGNED_WORKSPACE_STACK_ID),
            Some(&vec![11, 33, 22])
        );
    }

    #[test]
    fn ordered_surfaces_preserves_unknown_relative_order() {
        let stacking = WindowStackingState {
            workspaces: BTreeMap::from([(1, vec![30, 10, 20]), (2, vec![200, 100])]),
        };

        let ordered =
            stacking.ordered_surfaces(vec![(1, 40), (1, 10), (1, 30), (2, 300), (2, 100)]);

        assert_eq!(ordered, vec![30, 10, 40, 100, 300]);
    }

    #[test]
    fn retain_known_prunes_destroyed_and_moved_surfaces() {
        let mut stacking =
            WindowStackingState { workspaces: BTreeMap::from([(1, vec![7, 8]), (2, vec![9])]) };
        let known = BTreeMap::from([(7, 1), (8, 2), (9, 2)]);

        stacking.retain_known(&known);

        assert_eq!(stacking.workspaces.get(&1), Some(&vec![7]));
        assert_eq!(stacking.workspaces.get(&2), Some(&vec![9]));
    }
}
