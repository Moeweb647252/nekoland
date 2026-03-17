use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::RenderSceneRole;

/// One graph-local render target identifier.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct RenderTargetId(pub u64);

/// One graph-local render pass identifier.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct RenderPassId(pub u64);

/// One execution target referenced by render passes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderTargetKind {
    OutputSwapchain(OutputId),
    Offscreen,
}

/// Broad execution categories used by the backend execution graph.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderPassKind {
    #[default]
    Scene,
    Composite,
    Readback,
}

/// One render pass node in the output-local execution graph.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderPassNode {
    pub kind: RenderPassKind,
    pub scene_role: RenderSceneRole,
    pub input_targets: Vec<RenderTargetId>,
    pub output_target: RenderTargetId,
    pub dependencies: Vec<RenderPassId>,
    pub item_indices: Vec<usize>,
}

impl RenderPassNode {
    pub fn scene(
        scene_role: RenderSceneRole,
        output_target: RenderTargetId,
        dependencies: Vec<RenderPassId>,
        item_indices: Vec<usize>,
    ) -> Self {
        Self {
            kind: RenderPassKind::Scene,
            scene_role,
            input_targets: Vec::new(),
            output_target,
            dependencies,
            item_indices,
        }
    }
}

/// One output-local execution graph plus deterministic traversal order.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputExecutionPlan {
    pub targets: BTreeMap<RenderTargetId, RenderTargetKind>,
    pub passes: BTreeMap<RenderPassId, RenderPassNode>,
    pub ordered_passes: Vec<RenderPassId>,
    pub terminal_passes: Vec<RenderPassId>,
}

impl OutputExecutionPlan {
    pub fn validate_acyclic(&self) -> bool {
        let mut indegree = self
            .passes
            .keys()
            .copied()
            .map(|pass_id| (pass_id, 0_usize))
            .collect::<BTreeMap<_, _>>();
        let mut dependents = self
            .passes
            .keys()
            .copied()
            .map(|pass_id| (pass_id, Vec::new()))
            .collect::<BTreeMap<_, Vec<RenderPassId>>>();

        for (pass_id, pass) in &self.passes {
            for dependency in &pass.dependencies {
                let Some(entry) = indegree.get_mut(pass_id) else {
                    return false;
                };
                if !self.passes.contains_key(dependency) {
                    return false;
                }
                *entry = entry.saturating_add(1);
                dependents.entry(*dependency).or_default().push(*pass_id);
            }
        }

        let mut queue = indegree
            .iter()
            .filter_map(|(pass_id, indegree)| (*indegree == 0).then_some(*pass_id))
            .collect::<VecDeque<_>>();
        let mut visited = 0_usize;

        while let Some(pass_id) = queue.pop_front() {
            visited = visited.saturating_add(1);
            if let Some(pass_dependents) = dependents.get(&pass_id) {
                for dependent in pass_dependents {
                    let Some(indegree) = indegree.get_mut(dependent) else {
                        return false;
                    };
                    *indegree = indegree.saturating_sub(1);
                    if *indegree == 0 {
                        queue.push_back(*dependent);
                    }
                }
            }
        }

        visited == self.passes.len()
    }

    pub fn reachable_passes(&self) -> BTreeSet<RenderPassId> {
        let mut reachable = BTreeSet::new();
        let mut stack = self.terminal_passes.clone();

        while let Some(pass_id) = stack.pop() {
            if !reachable.insert(pass_id) {
                continue;
            }
            if let Some(pass) = self.passes.get(&pass_id) {
                stack.extend(pass.dependencies.iter().copied());
            }
        }

        reachable
    }

    pub fn reachable_passes_in_order(&self) -> Vec<RenderPassId> {
        let reachable = self.reachable_passes();
        self.ordered_passes
            .iter()
            .copied()
            .filter(|pass_id| reachable.contains(pass_id))
            .collect::<Vec<_>>()
    }
}

/// Output-scoped backend execution graph derived from the current render plan.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderPassGraph {
    pub outputs: BTreeMap<OutputId, OutputExecutionPlan>,
}

impl RenderPassGraph {
    pub fn validate_acyclic(&self) -> bool {
        self.outputs.values().all(OutputExecutionPlan::validate_acyclic)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::components::OutputId;
    use crate::resources::{
        OutputExecutionPlan, RenderPassGraph, RenderPassId, RenderPassKind, RenderPassNode,
        RenderSceneRole, RenderTargetId, RenderTargetKind,
    };

    #[test]
    fn output_execution_plan_detects_cycles() {
        let cyclic = OutputExecutionPlan {
            targets: BTreeMap::from([(
                RenderTargetId(1),
                RenderTargetKind::OutputSwapchain(OutputId(1)),
            )]),
            passes: BTreeMap::from([
                (
                    RenderPassId(1),
                    RenderPassNode {
                        kind: RenderPassKind::Scene,
                        scene_role: RenderSceneRole::Desktop,
                        input_targets: Vec::new(),
                        output_target: RenderTargetId(1),
                        dependencies: vec![RenderPassId(2)],
                        item_indices: vec![0],
                    },
                ),
                (
                    RenderPassId(2),
                    RenderPassNode {
                        kind: RenderPassKind::Composite,
                        scene_role: RenderSceneRole::Compositor,
                        input_targets: vec![RenderTargetId(1)],
                        output_target: RenderTargetId(1),
                        dependencies: vec![RenderPassId(1)],
                        item_indices: Vec::new(),
                    },
                ),
            ]),
            ordered_passes: vec![RenderPassId(1), RenderPassId(2)],
            terminal_passes: vec![RenderPassId(2)],
        };

        assert!(!cyclic.validate_acyclic());
    }

    #[test]
    fn reachable_passes_follow_terminal_dependencies() {
        let plan = OutputExecutionPlan {
            targets: BTreeMap::from([(
                RenderTargetId(1),
                RenderTargetKind::OutputSwapchain(OutputId(1)),
            )]),
            passes: BTreeMap::from([
                (
                    RenderPassId(1),
                    RenderPassNode::scene(
                        RenderSceneRole::Desktop,
                        RenderTargetId(1),
                        Vec::new(),
                        vec![0],
                    ),
                ),
                (
                    RenderPassId(2),
                    RenderPassNode::scene(
                        RenderSceneRole::Overlay,
                        RenderTargetId(1),
                        vec![RenderPassId(1)],
                        vec![1],
                    ),
                ),
                (
                    RenderPassId(3),
                    RenderPassNode::scene(
                        RenderSceneRole::Cursor,
                        RenderTargetId(1),
                        Vec::new(),
                        vec![2],
                    ),
                ),
            ]),
            ordered_passes: vec![RenderPassId(1), RenderPassId(2), RenderPassId(3)],
            terminal_passes: vec![RenderPassId(2)],
        };

        assert!(plan.validate_acyclic());
        assert_eq!(plan.reachable_passes_in_order(), vec![RenderPassId(1), RenderPassId(2)],);
    }

    #[test]
    fn render_pass_graph_validates_all_outputs() {
        let graph = RenderPassGraph {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputExecutionPlan {
                    targets: BTreeMap::from([(
                        RenderTargetId(1),
                        RenderTargetKind::OutputSwapchain(OutputId(7)),
                    )]),
                    passes: BTreeMap::from([(
                        RenderPassId(1),
                        RenderPassNode::scene(
                            RenderSceneRole::Desktop,
                            RenderTargetId(1),
                            Vec::new(),
                            vec![0],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                },
            )]),
        };

        assert!(graph.validate_acyclic());
    }
}
