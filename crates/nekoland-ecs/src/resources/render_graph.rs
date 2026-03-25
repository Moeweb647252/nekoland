//! Backend-neutral execution graphs compiled from output-local render plans.

#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::OutputId;
use crate::resources::{ProcessRect, RenderItemId, RenderSceneRole, ScreenshotRequestId};

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
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderTargetKind {
    #[default]
    OffscreenColor,
    OutputSwapchain(OutputId),
    OffscreenIntermediate,
}

/// Broad execution categories used by the backend execution graph.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderPassKind {
    #[default]
    Scene,
    Composite,
    PostProcess,
    Readback,
}

/// Stable material-like identifier for one post-process implementation.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct RenderMaterialId(pub u64);

/// Stable frame-local handle for one material parameter payload.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(transparent)]
pub struct MaterialParamsId(pub u64);

/// Scene-pass payload referencing render-plan item ids.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenePassConfig {
    pub item_ids: Vec<RenderItemId>,
}

/// Composite-pass payload copying one source target into the destination target.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositePassConfig {
    pub source_target: RenderTargetId,
}

/// Post-process-pass payload transforming one source target into another target.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostProcessPassConfig {
    pub source_target: RenderTargetId,
    pub material_id: RenderMaterialId,
    pub params_id: Option<MaterialParamsId>,
    pub process_regions: Vec<ProcessRect>,
}

/// Readback-pass payload exposing one source target for future capture/readback paths.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadbackPassConfig {
    pub source_target: RenderTargetId,
    pub request_ids: Vec<ScreenshotRequestId>,
}

/// Concrete payload carried by one execution-graph pass node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RenderPassPayload {
    Scene(ScenePassConfig),
    Composite(CompositePassConfig),
    PostProcess(PostProcessPassConfig),
    Readback(ReadbackPassConfig),
}

/// One render pass node in the output-local execution graph.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RenderPassNode {
    pub kind: RenderPassKind,
    pub scene_role: RenderSceneRole,
    pub input_targets: Vec<RenderTargetId>,
    pub output_target: RenderTargetId,
    pub dependencies: Vec<RenderPassId>,
    pub payload: RenderPassPayload,
}

impl RenderPassNode {
    /// Builds a scene pass that directly rasterizes ordered render items.
    pub fn scene(
        scene_role: RenderSceneRole,
        output_target: RenderTargetId,
        dependencies: Vec<RenderPassId>,
        item_ids: Vec<RenderItemId>,
    ) -> Self {
        Self {
            kind: RenderPassKind::Scene,
            scene_role,
            input_targets: Vec::new(),
            output_target,
            dependencies,
            payload: RenderPassPayload::Scene(ScenePassConfig { item_ids }),
        }
    }

    /// Builds a composite pass that copies one target into another target.
    pub fn composite(
        scene_role: RenderSceneRole,
        source_target: RenderTargetId,
        output_target: RenderTargetId,
        dependencies: Vec<RenderPassId>,
    ) -> Self {
        Self {
            kind: RenderPassKind::Composite,
            scene_role,
            input_targets: vec![source_target],
            output_target,
            dependencies,
            payload: RenderPassPayload::Composite(CompositePassConfig { source_target }),
        }
    }

    /// Builds a post-process pass driven by one material descriptor and optional params.
    pub fn post_process(
        scene_role: RenderSceneRole,
        source_target: RenderTargetId,
        output_target: RenderTargetId,
        dependencies: Vec<RenderPassId>,
        material_id: RenderMaterialId,
        params_id: Option<MaterialParamsId>,
        process_regions: Vec<ProcessRect>,
    ) -> Self {
        Self {
            kind: RenderPassKind::PostProcess,
            scene_role,
            input_targets: vec![source_target],
            output_target,
            dependencies,
            payload: RenderPassPayload::PostProcess(PostProcessPassConfig {
                source_target,
                material_id,
                params_id,
                process_regions,
            }),
        }
    }

    /// Builds a readback pass that exposes one target to screenshot capture.
    pub fn readback(
        scene_role: RenderSceneRole,
        source_target: RenderTargetId,
        output_target: RenderTargetId,
        dependencies: Vec<RenderPassId>,
        request_ids: Vec<ScreenshotRequestId>,
    ) -> Self {
        Self {
            kind: RenderPassKind::Readback,
            scene_role,
            input_targets: vec![source_target],
            output_target,
            dependencies,
            payload: RenderPassPayload::Readback(ReadbackPassConfig { source_target, request_ids }),
        }
    }

    /// Returns render-plan item ids only for scene passes.
    pub fn item_ids(&self) -> &[RenderItemId] {
        match &self.payload {
            RenderPassPayload::Scene(config) => &config.item_ids,
            RenderPassPayload::Composite(_)
            | RenderPassPayload::PostProcess(_)
            | RenderPassPayload::Readback(_) => &[],
        }
    }
}

/// One output-local execution graph plus deterministic traversal order.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputExecutionPlan {
    pub targets: BTreeMap<RenderTargetId, RenderTargetKind>,
    pub passes: BTreeMap<RenderPassId, RenderPassNode>,
    pub ordered_passes: Vec<RenderPassId>,
    pub terminal_passes: Vec<RenderPassId>,
}

impl OutputExecutionPlan {
    /// Validates that the output-local pass graph contains no cycles.
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

    /// Returns the set of passes reachable from terminal passes.
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

    /// Returns reachable passes filtered into deterministic execution order.
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
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RenderPassGraph {
    pub outputs: BTreeMap<OutputId, OutputExecutionPlan>,
}

impl RenderPassGraph {
    /// Validates that every output-local execution graph is acyclic.
    pub fn validate_acyclic(&self) -> bool {
        self.outputs.values().all(OutputExecutionPlan::validate_acyclic)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::components::OutputId;
    use crate::resources::{
        MaterialParamsId, OutputExecutionPlan, RenderItemId, RenderMaterialId, RenderPassGraph,
        RenderPassId, RenderPassKind, RenderPassNode, RenderPassPayload, RenderSceneRole,
        RenderTargetId, RenderTargetKind,
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
                        payload: RenderPassPayload::Scene(super::ScenePassConfig {
                            item_ids: vec![RenderItemId(1)],
                        }),
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
                        payload: RenderPassPayload::Composite(super::CompositePassConfig {
                            source_target: RenderTargetId(1),
                        }),
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
                        vec![RenderItemId(0)],
                    ),
                ),
                (
                    RenderPassId(2),
                    RenderPassNode::scene(
                        RenderSceneRole::Overlay,
                        RenderTargetId(1),
                        vec![RenderPassId(1)],
                        vec![RenderItemId(1)],
                    ),
                ),
                (
                    RenderPassId(3),
                    RenderPassNode::scene(
                        RenderSceneRole::Cursor,
                        RenderTargetId(1),
                        Vec::new(),
                        vec![RenderItemId(2)],
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
                            vec![RenderItemId(0)],
                        ),
                    )]),
                    ordered_passes: vec![RenderPassId(1)],
                    terminal_passes: vec![RenderPassId(1)],
                },
            )]),
        };

        assert!(graph.validate_acyclic());
    }

    #[test]
    fn post_process_node_carries_material_and_params() {
        let node = RenderPassNode::post_process(
            RenderSceneRole::Desktop,
            RenderTargetId(1),
            RenderTargetId(2),
            vec![RenderPassId(3)],
            RenderMaterialId(7),
            Some(MaterialParamsId(9)),
            Vec::new(),
        );

        assert_eq!(node.kind, RenderPassKind::PostProcess);
        assert_eq!(node.input_targets, vec![RenderTargetId(1)]);
        match node.payload {
            RenderPassPayload::PostProcess(config) => {
                assert_eq!(config.material_id, RenderMaterialId(7));
                assert_eq!(config.params_id, Some(MaterialParamsId(9)));
            }
            _ => panic!("expected post-process payload"),
        }
    }
}
