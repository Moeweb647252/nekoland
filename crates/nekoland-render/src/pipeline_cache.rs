//! Pipeline specialization keys and cache-state projection derived from render plans.
//!
//! This module mostly contains render-internal key/value data structures. The module-level
//! description is the important part; variant and field names intentionally stay lightweight.
#![allow(missing_docs)]

use std::collections::BTreeSet;

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::resources::{
    ProcessShaderKey, RenderPassGraph, RenderPlan, RenderPlanItem, RenderSceneRole, RenderTargetId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderColorFormat {
    Rgba8Unorm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderBlendMode {
    Replace,
    AlphaBlend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderSampleMode {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RenderClipMode {
    None,
    ClipRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScenePipelineDrawKind {
    Surface,
    Quad,
    Text,
    Backdrop,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScenePipelineKey {
    pub draw_kind: ScenePipelineDrawKind,
    pub scene_role: RenderSceneRole,
    pub color_format: RenderColorFormat,
    pub blend_mode: RenderBlendMode,
    pub sample_mode: RenderSampleMode,
    pub clip_mode: RenderClipMode,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessPipelineKey {
    pub shader_key: ProcessShaderKey,
    pub pass_role: ProcessPassRole,
    pub color_format: RenderColorFormat,
    pub sample_mode: RenderSampleMode,
    pub blend_mode: RenderBlendMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProcessPassRole {
    Composite,
    PostProcess,
}

#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderPipelineCacheState {
    pub scene_pipelines: BTreeSet<ScenePipelineKey>,
    pub process_pipelines: BTreeSet<ProcessPipelineKey>,
    pub readback_targets: BTreeSet<RenderTargetId>,
}

/// Projects scene-draw pipeline requirements out of the current render plan and graph.
pub fn build_render_pipeline_cache_state_system(
    render_plan: Res<'_, RenderPlan>,
    render_graph: Res<'_, RenderPassGraph>,
    mut cache: ResMut<'_, RenderPipelineCacheState>,
) {
    let mut scene_pipelines = BTreeSet::new();
    let mut process_pipelines = BTreeSet::new();
    let mut readback_targets = BTreeSet::new();

    for output_plan in render_plan.outputs.values() {
        for item in output_plan.iter_ordered() {
            scene_pipelines.insert(scene_pipeline_key_for_item(item));
        }
    }

    for execution in render_graph.outputs.values() {
        for pass in execution.passes.values() {
            if matches!(pass.kind, nekoland_ecs::resources::RenderPassKind::Readback) {
                readback_targets.insert(pass.output_target);
            }
            if matches!(pass.payload, nekoland_ecs::resources::RenderPassPayload::Composite(_)) {
                process_pipelines.insert(ProcessPipelineKey {
                    shader_key: ProcessShaderKey::BuiltinComposite,
                    pass_role: ProcessPassRole::Composite,
                    color_format: RenderColorFormat::Rgba8Unorm,
                    sample_mode: RenderSampleMode::Linear,
                    blend_mode: RenderBlendMode::Replace,
                });
            }
        }
    }

    cache.scene_pipelines = scene_pipelines;
    cache.process_pipelines = process_pipelines;
    cache.readback_targets = readback_targets;
}

/// Projects process-pass pipeline requirements out of the current process plan.
pub fn build_process_pipeline_cache_state_system(
    process_plan: Res<'_, nekoland_ecs::resources::RenderProcessPlan>,
    mut cache: ResMut<'_, RenderPipelineCacheState>,
) {
    for output in process_plan.outputs.values() {
        for unit in output.units.values() {
            match &unit.shader_key {
                ProcessShaderKey::BuiltinComposite => {
                    cache.process_pipelines.insert(ProcessPipelineKey {
                        shader_key: ProcessShaderKey::BuiltinComposite,
                        pass_role: ProcessPassRole::Composite,
                        color_format: RenderColorFormat::Rgba8Unorm,
                        sample_mode: RenderSampleMode::Linear,
                        blend_mode: RenderBlendMode::Replace,
                    });
                }
                ProcessShaderKey::Material(key) => {
                    cache.process_pipelines.insert(ProcessPipelineKey {
                        shader_key: ProcessShaderKey::Material(key.clone()),
                        pass_role: ProcessPassRole::PostProcess,
                        color_format: RenderColorFormat::Rgba8Unorm,
                        sample_mode: RenderSampleMode::Linear,
                        blend_mode: RenderBlendMode::AlphaBlend,
                    });
                }
                ProcessShaderKey::Passthrough => {}
            }
        }
    }
}

fn scene_pipeline_key_for_item(item: &RenderPlanItem) -> ScenePipelineKey {
    let instance = item.instance();
    let clip_mode =
        if instance.clip_rect.is_some() { RenderClipMode::ClipRect } else { RenderClipMode::None };
    match item {
        RenderPlanItem::Surface(_) => ScenePipelineKey {
            draw_kind: ScenePipelineDrawKind::Surface,
            scene_role: instance.scene_role,
            color_format: RenderColorFormat::Rgba8Unorm,
            blend_mode: RenderBlendMode::AlphaBlend,
            sample_mode: RenderSampleMode::Linear,
            clip_mode,
        },
        RenderPlanItem::Quad(item) => ScenePipelineKey {
            draw_kind: ScenePipelineDrawKind::Quad,
            scene_role: instance.scene_role,
            color_format: RenderColorFormat::Rgba8Unorm,
            blend_mode: RenderBlendMode::AlphaBlend,
            sample_mode: match &item.content {
                nekoland_ecs::resources::QuadContent::SolidColor { .. } => {
                    RenderSampleMode::Nearest
                }
                nekoland_ecs::resources::QuadContent::RasterImage { .. } => {
                    RenderSampleMode::Linear
                }
            },
            clip_mode,
        },
        RenderPlanItem::Text(_) => ScenePipelineKey {
            draw_kind: ScenePipelineDrawKind::Text,
            scene_role: instance.scene_role,
            color_format: RenderColorFormat::Rgba8Unorm,
            blend_mode: RenderBlendMode::AlphaBlend,
            sample_mode: RenderSampleMode::Linear,
            clip_mode,
        },
        RenderPlanItem::Backdrop(_) => ScenePipelineKey {
            draw_kind: ScenePipelineDrawKind::Backdrop,
            scene_role: instance.scene_role,
            color_format: RenderColorFormat::Rgba8Unorm,
            blend_mode: RenderBlendMode::AlphaBlend,
            sample_mode: RenderSampleMode::Linear,
            clip_mode,
        },
        RenderPlanItem::Cursor(_) => ScenePipelineKey {
            draw_kind: ScenePipelineDrawKind::Cursor,
            scene_role: instance.scene_role,
            color_format: RenderColorFormat::Rgba8Unorm,
            blend_mode: RenderBlendMode::AlphaBlend,
            sample_mode: RenderSampleMode::Linear,
            clip_mode,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        BackdropRenderItem, OutputRenderPlan, ProcessShaderKey, RenderItemId, RenderItemIdentity,
        RenderItemInstance, RenderMaterialKind, RenderPassGraph, RenderPlan, RenderPlanItem,
        RenderProcessPlan, RenderRect, RenderSceneRole, RenderSourceId, RenderTargetId,
    };

    use super::{
        ProcessPassRole, ProcessPipelineKey, RenderBlendMode, RenderClipMode, RenderColorFormat,
        RenderPipelineCacheState, RenderSampleMode, ScenePipelineDrawKind,
        build_process_pipeline_cache_state_system, build_render_pipeline_cache_state_system,
    };

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[test]
    fn scene_pipeline_cache_tracks_draw_specialization_keys() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(1),
                OutputRenderPlan::from_items([
                    RenderPlanItem::Backdrop(BackdropRenderItem {
                        identity: identity(1),
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 0,
                            scene_role: RenderSceneRole::Compositor,
                        },
                    }),
                    RenderPlanItem::Backdrop(BackdropRenderItem {
                        identity: identity(2),
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 50, height: 50 },
                            opacity: 1.0,
                            clip_rect: Some(RenderRect { x: 10, y: 10, width: 20, height: 20 }),
                            z_index: 1,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    }),
                ]),
            )]),
        });
        world.insert_resource(RenderPassGraph::default());
        world.init_resource::<RenderPipelineCacheState>();

        let mut system = IntoSystem::into_system(build_render_pipeline_cache_state_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let cache = world.resource::<RenderPipelineCacheState>();
        assert!(cache.scene_pipelines.iter().any(|key| {
            key.draw_kind == ScenePipelineDrawKind::Backdrop
                && key.scene_role == RenderSceneRole::Compositor
                && key.clip_mode == RenderClipMode::None
                && key.sample_mode == RenderSampleMode::Linear
        }));
        assert!(cache.scene_pipelines.iter().any(|key| {
            key.draw_kind == ScenePipelineDrawKind::Backdrop
                && key.scene_role == RenderSceneRole::Overlay
                && key.clip_mode == RenderClipMode::ClipRect
                && key.blend_mode == RenderBlendMode::AlphaBlend
        }));
    }

    #[test]
    fn process_pipeline_cache_tracks_builtin_and_material_pipelines() {
        let mut world = World::default();
        world.insert_resource(RenderProcessPlan {
            outputs: BTreeMap::from([(
                OutputId(1),
                nekoland_ecs::resources::OutputProcessPlan {
                    units: BTreeMap::from([
                        (
                            nekoland_ecs::resources::ProcessUnitId(1),
                            nekoland_ecs::resources::ProcessUnit {
                                shader_key: ProcessShaderKey::BuiltinComposite,
                                input: nekoland_ecs::resources::ProcessInputRef {
                                    target_id: RenderTargetId(1),
                                    sample_rect: None,
                                },
                                output: nekoland_ecs::resources::ProcessTargetRef {
                                    target_id: RenderTargetId(2),
                                    output_rect: None,
                                },
                                uniforms: nekoland_ecs::resources::ProcessUniformBlock::default(),
                                process_regions: Vec::new(),
                            },
                        ),
                        (
                            nekoland_ecs::resources::ProcessUnitId(2),
                            nekoland_ecs::resources::ProcessUnit {
                                shader_key: ProcessShaderKey::Material(
                                    nekoland_ecs::resources::RenderMaterialPipelineKey::post_process(
                                        RenderMaterialKind::BackdropBlur,
                                    ),
                                ),
                                input: nekoland_ecs::resources::ProcessInputRef {
                                    target_id: RenderTargetId(2),
                                    sample_rect: None,
                                },
                                output: nekoland_ecs::resources::ProcessTargetRef {
                                    target_id: RenderTargetId(3),
                                    output_rect: None,
                                },
                                uniforms: nekoland_ecs::resources::ProcessUniformBlock::default(),
                                process_regions: Vec::new(),
                            },
                        ),
                    ]),
                    ordered_units: vec![
                        nekoland_ecs::resources::ProcessUnitId(1),
                        nekoland_ecs::resources::ProcessUnitId(2),
                    ],
                    pass_units: BTreeMap::default(),
                },
            )]),
        });
        world.init_resource::<RenderPipelineCacheState>();

        let mut system = IntoSystem::into_system(build_process_pipeline_cache_state_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let cache = world.resource::<RenderPipelineCacheState>();
        assert!(cache.process_pipelines.contains(&ProcessPipelineKey {
            shader_key: ProcessShaderKey::BuiltinComposite,
            pass_role: ProcessPassRole::Composite,
            color_format: RenderColorFormat::Rgba8Unorm,
            sample_mode: RenderSampleMode::Linear,
            blend_mode: RenderBlendMode::Replace,
        }));
        assert!(cache.process_pipelines.contains(&ProcessPipelineKey {
            shader_key: ProcessShaderKey::Material(
                nekoland_ecs::resources::RenderMaterialPipelineKey::post_process(
                    RenderMaterialKind::BackdropBlur,
                ),
            ),
            pass_role: ProcessPassRole::PostProcess,
            color_format: RenderColorFormat::Rgba8Unorm,
            sample_mode: RenderSampleMode::Linear,
            blend_mode: RenderBlendMode::AlphaBlend,
        }));
    }
}
