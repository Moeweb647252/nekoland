//! Preparation of scene and GPU descriptors used by backend executors.
//!
//! These resources are still backend-neutral: they describe what needs to be imported, allocated,
//! or bound, but they do not perform live GPU work themselves.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::components::WlSurfaceHandle;
use nekoland_ecs::resources::{
    CursorRenderSource, OutputPreparedGpuResources, OutputPreparedSceneResources,
    OutputTargetAllocationPlan, PlatformSurfaceImportStrategy, PlatformSurfaceSnapshotState,
    PreparedBackdropSceneItem, PreparedGpuResources, PreparedMaterialBinding,
    PreparedMaterialBindingCacheKey, PreparedMaterialBindingKey, PreparedNamedCursorSceneItem,
    PreparedQuadSceneItem, PreparedRenderTargetCacheKey, PreparedRenderTargetResource,
    PreparedSceneItem, PreparedSceneResources, PreparedSurfaceCursorSceneItem,
    PreparedSurfaceImport, PreparedSurfaceImportCacheKey, PreparedSurfaceImportStrategy,
    PreparedSurfaceSceneItem, RenderMaterialFrameState, RenderPassGraph, RenderPlan,
    RenderProcessPlan, RenderTargetAllocationPlan, RenderTargetAllocationSpec, ShellRenderInput,
    SurfaceBufferAttachmentSnapshot, SurfaceBufferAttachmentState, SurfaceTextureBridgePlan,
    SurfaceTextureImportDescriptor,
};

use crate::compositor_render::RenderViewSnapshot;
use crate::material::RenderMaterialRequestQueue;

/// Snapshot each surface's current buffer attachment state for later import planning.
///
/// The render plan only references surfaces by id. This system captures whether a buffer is
/// currently attached, plus the scale that buffer advertises, so later preparation stages can
/// decide whether an import can be scheduled.
pub fn sync_surface_buffer_attachment_snapshot_system(
    surfaces: bevy_ecs::prelude::Query<
        '_,
        '_,
        (&'static WlSurfaceHandle, &'static nekoland_ecs::components::BufferState),
    >,
    mut snapshot: ResMut<'_, SurfaceBufferAttachmentSnapshot>,
) {
    snapshot.surfaces = surfaces
        .iter()
        .map(|(surface, buffer)| {
            (
                surface.id,
                SurfaceBufferAttachmentState { attached: buffer.attached, scale: buffer.scale },
            )
        })
        .collect();
}

/// Derive per-output render target sizes from the compiled pass graph and current output views.
///
/// Targets stay backend-neutral here. The plan only states which targets each output needs and the
/// dimensions they must match for the current frame.
pub fn build_render_target_allocation_plan_system(
    views: Res<'_, RenderViewSnapshot>,
    render_graph: Res<'_, RenderPassGraph>,
    mut plan: ResMut<'_, RenderTargetAllocationPlan>,
) {
    plan.outputs = render_graph
        .outputs
        .iter()
        .map(|(output_id, execution)| {
            let Some(view) = views.view(*output_id) else {
                return (*output_id, OutputTargetAllocationPlan::default());
            };
            let targets = execution
                .targets
                .iter()
                .map(|(target_id, kind)| {
                    (
                        *target_id,
                        RenderTargetAllocationSpec {
                            kind: kind.clone(),
                            width: view.width.max(1),
                            height: view.height.max(1),
                        },
                    )
                })
                .collect();
            (*output_id, OutputTargetAllocationPlan { targets })
        })
        .collect();
}

/// Gather all surfaces referenced by the render plan into one import descriptor table.
///
/// This bridges shell presentation state, platform surface snapshots, and render-plan usage into a
/// single backend-facing description of how each surface should be imported for the frame.
pub fn build_surface_texture_bridge_plan_system(
    render_plan: Res<'_, RenderPlan>,
    shell_render_input: Res<'_, ShellRenderInput>,
    buffer_state: Res<'_, SurfaceBufferAttachmentSnapshot>,
    surface_snapshots: Option<Res<'_, PlatformSurfaceSnapshotState>>,
    mut bridge: ResMut<'_, SurfaceTextureBridgePlan>,
) {
    let surface_presentation = Some(&shell_render_input.surface_presentation);
    let surface_snapshots = surface_snapshots.as_deref();
    let mut surfaces = BTreeMap::<u64, SurfaceTextureImportDescriptor>::new();

    for (output_id, output_plan) in &render_plan.outputs {
        for item in output_plan.iter_ordered() {
            let Some(surface_id) = item.surface_id() else {
                continue;
            };
            let state =
                surface_presentation.and_then(|snapshot| snapshot.surfaces.get(&surface_id));
            let buffer = buffer_state.surfaces.get(&surface_id).copied().unwrap_or_default();
            let descriptor =
                surfaces.entry(surface_id).or_insert_with(|| SurfaceTextureImportDescriptor {
                    surface_id,
                    surface_kind: surface_snapshots
                        .map(|surface_snapshots| surface_snapshots.kind(surface_id))
                        .unwrap_or_default(),
                    buffer_source: surface_snapshots
                        .map(|surface_snapshots| surface_snapshots.buffer_source(surface_id))
                        .unwrap_or_default(),
                    dmabuf_format: surface_snapshots
                        .and_then(|surface_snapshots| surface_snapshots.dmabuf_format(surface_id)),
                    import_strategy: surface_snapshots
                        .map(|surface_snapshots| surface_snapshots.import_strategy(surface_id))
                        .unwrap_or_default(),
                    target_outputs: BTreeSet::new(),
                    content_version: surface_snapshots
                        .map(|surface_snapshots| surface_snapshots.content_version(surface_id))
                        .unwrap_or_default(),
                    attached: buffer.attached,
                    scale: buffer.scale,
                });
            descriptor.target_outputs.insert(*output_id);
            if let Some(state) = state {
                descriptor.attached = descriptor.attached || state.visible;
            }
        }
    }

    bridge.surfaces = surfaces;
}

/// Convert ordered render-plan items into backend-neutral scene descriptors.
///
/// The result keeps per-output draw ordering while replacing world-facing item variants with
/// stable prepared forms that the backend can consume directly.
pub fn build_prepared_scene_resources_system(
    views: Res<'_, RenderViewSnapshot>,
    render_plan: Res<'_, RenderPlan>,
    surface_bridge: Res<'_, SurfaceTextureBridgePlan>,
    mut prepared: ResMut<'_, PreparedSceneResources>,
) {
    prepared.outputs = render_plan
        .outputs
        .iter()
        .map(|(output_id, output_plan)| {
            let scale = views.view(*output_id).map(|view| view.scale.max(1)).unwrap_or(1);
            let mut items = BTreeMap::new();
            let mut ordered_items = Vec::new();

            for item in output_plan.iter_ordered() {
                let Some(prepared_item) =
                    prepare_scene_item_descriptor(item, scale, &surface_bridge)
                else {
                    continue;
                };
                items.insert(item.item_id(), prepared_item);
                ordered_items.push(item.item_id());
            }

            (*output_id, OutputPreparedSceneResources { items, ordered_items })
        })
        .collect();
}

fn prepare_scene_item_descriptor(
    item: &nekoland_ecs::resources::RenderPlanItem,
    output_scale: u32,
    surface_bridge: &SurfaceTextureBridgePlan,
) -> Option<PreparedSceneItem> {
    match item {
        nekoland_ecs::resources::RenderPlanItem::Surface(item) => {
            let visible_rect = item.instance.visible_rect()?;
            let bridge = surface_bridge.surfaces.get(&item.surface_id);
            Some(PreparedSceneItem::Surface(PreparedSurfaceSceneItem {
                surface_id: item.surface_id,
                surface_kind: bridge.map(|bridge| bridge.surface_kind).unwrap_or_default(),
                x: item.instance.rect.x,
                y: item.instance.rect.y,
                visible_rect,
                opacity: item.instance.opacity,
                import_ready: bridge.is_some_and(|bridge| bridge.attached),
            }))
        }
        nekoland_ecs::resources::RenderPlanItem::Quad(item) => {
            let visible_rect = item.instance.visible_rect()?;
            Some(PreparedSceneItem::Quad(PreparedQuadSceneItem {
                rect: item.instance.rect,
                visible_rect,
                content: item.content.clone(),
                opacity: item.instance.opacity,
            }))
        }
        nekoland_ecs::resources::RenderPlanItem::Backdrop(item) => {
            let visible_rect = item.instance.visible_rect()?;
            Some(PreparedSceneItem::Backdrop(PreparedBackdropSceneItem { visible_rect }))
        }
        nekoland_ecs::resources::RenderPlanItem::Cursor(item) => match &item.source {
            CursorRenderSource::Named { icon_name } => {
                Some(PreparedSceneItem::CursorNamed(PreparedNamedCursorSceneItem {
                    icon_name: icon_name.clone(),
                    x: item.instance.rect.x,
                    y: item.instance.rect.y,
                    scale: output_scale.max(1),
                    opacity: item.instance.opacity,
                }))
            }
            CursorRenderSource::Surface { surface_id } => {
                let visible_rect = item.instance.visible_rect()?;
                let bridge = surface_bridge.surfaces.get(surface_id);
                Some(PreparedSceneItem::CursorSurface(PreparedSurfaceCursorSceneItem {
                    surface_id: *surface_id,
                    x: item.instance.rect.x,
                    y: item.instance.rect.y,
                    visible_rect,
                    opacity: item.instance.opacity,
                    import_ready: bridge.is_some_and(|bridge| bridge.attached),
                }))
            }
        },
    }
}

/// Assemble prepared GPU-side resources for each output from the frame's planning state.
///
/// This stage resolves render-target allocations, surface imports, material bindings, and custom
/// process shaders into one cache-friendly descriptor set without touching live backend objects.
pub fn build_prepared_gpu_resources_system(
    render_target_allocation: Res<'_, RenderTargetAllocationPlan>,
    surface_bridge: Res<'_, SurfaceTextureBridgePlan>,
    material_requests: Res<'_, RenderMaterialRequestQueue>,
    materials: Res<'_, RenderMaterialFrameState>,
    process_plan: Res<'_, RenderProcessPlan>,
    mut prepared: ResMut<'_, PreparedGpuResources>,
) {
    prepared.surface_imports = surface_bridge
        .surfaces
        .iter()
        .map(|(surface_id, descriptor)| {
            (
                *surface_id,
                PreparedSurfaceImport {
                    surface_id: *surface_id,
                    descriptor: descriptor.clone(),
                    strategy: prepared_surface_import_strategy(descriptor.import_strategy),
                    cache_key: PreparedSurfaceImportCacheKey {
                        surface_id: *surface_id,
                        content_version: descriptor.content_version,
                        strategy: prepared_surface_import_strategy(descriptor.import_strategy),
                    },
                },
            )
        })
        .collect();

    prepared.material_bindings.clear();

    let output_ids = render_target_allocation
        .outputs
        .keys()
        .copied()
        .chain(material_requests.outputs.keys().copied())
        .chain(
            surface_bridge
                .surfaces
                .values()
                .flat_map(|descriptor| descriptor.target_outputs.iter().copied()),
        )
        .collect::<BTreeSet<_>>();

    prepared.outputs = output_ids
        .into_iter()
        .map(|output_id| {
            let targets = render_target_allocation
                .outputs
                .get(&output_id)
                .map(|allocation| {
                    allocation
                        .targets
                        .iter()
                        .map(|(target_id, spec)| {
                            (
                                *target_id,
                                PreparedRenderTargetResource {
                                    target_id: *target_id,
                                    kind: spec.kind.clone(),
                                    width: spec.width,
                                    height: spec.height,
                                    cache_key: PreparedRenderTargetCacheKey {
                                        output_id,
                                        target_id: *target_id,
                                        kind: spec.kind.clone(),
                                        width: spec.width,
                                        height: spec.height,
                                    },
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();

            let surface_imports = prepared
                .surface_imports
                .iter()
                .filter_map(|(surface_id, import)| {
                    import
                        .descriptor
                        .target_outputs
                        .contains(&output_id)
                        .then_some((*surface_id, import.clone()))
                })
                .collect::<BTreeMap<_, _>>();

            let material_bindings = material_requests
                .outputs
                .get(&output_id)
                .map(|requests| {
                    requests
                        .iter()
                        .filter_map(|request| {
                            let descriptor = materials.descriptors.get(&request.material_id)?;
                            let key = PreparedMaterialBindingKey {
                                material_id: request.material_id,
                                params_id: request.params_id,
                            };
                            prepared.material_bindings.entry(key).or_insert_with(|| {
                                PreparedMaterialBinding {
                                    key,
                                    descriptor: descriptor.clone(),
                                    bind_group_layout: descriptor.bind_group_layout.clone(),
                                    params: request
                                        .params_id
                                        .and_then(|params_id| materials.params.get(&params_id))
                                        .cloned(),
                                    cache_key: PreparedMaterialBindingCacheKey {
                                        output_id,
                                        binding_key: key,
                                        descriptor: descriptor.clone(),
                                        bind_group_layout: descriptor.bind_group_layout.clone(),
                                        params: request
                                            .params_id
                                            .and_then(|params_id| materials.params.get(&params_id))
                                            .cloned(),
                                    },
                                }
                            });
                            Some(key)
                        })
                        .collect()
                })
                .unwrap_or_default();

            let process_shaders = process_plan
                .outputs
                .get(&output_id)
                .map(|output_process| {
                    output_process
                        .units
                        .values()
                        .map(|unit| unit.shader_key.clone())
                        .filter(|shader_key| {
                            !matches!(
                                shader_key,
                                nekoland_ecs::resources::ProcessShaderKey::Passthrough
                                    | nekoland_ecs::resources::ProcessShaderKey::BuiltinComposite
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();

            (
                output_id,
                OutputPreparedGpuResources {
                    targets,
                    surface_imports,
                    process_shaders,
                    material_bindings,
                },
            )
        })
        .collect();
}

fn prepared_surface_import_strategy(
    import_strategy: PlatformSurfaceImportStrategy,
) -> PreparedSurfaceImportStrategy {
    match import_strategy {
        PlatformSurfaceImportStrategy::ShmUpload => PreparedSurfaceImportStrategy::ShmUpload,
        PlatformSurfaceImportStrategy::DmaBufImport => PreparedSurfaceImportStrategy::DmaBufImport,
        PlatformSurfaceImportStrategy::ExternalTextureImport => {
            PreparedSurfaceImportStrategy::ExternalTextureImport
        }
        PlatformSurfaceImportStrategy::SinglePixelFill => {
            PreparedSurfaceImportStrategy::SinglePixelFill
        }
        PlatformSurfaceImportStrategy::Unsupported => PreparedSurfaceImportStrategy::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        CursorRenderSource, MaterialParamsId, OutputExecutionPlan, OutputProcessPlan,
        OutputRenderPlan, PlatformSurfaceBufferSource, PlatformSurfaceKind,
        PlatformSurfaceSnapshot, PlatformSurfaceSnapshotState, PreparedGpuResources,
        PreparedSceneItem, ProcessInputRef, ProcessShaderKey, ProcessTargetRef,
        ProcessUniformBlock, ProcessUnit, ProcessUnitId, QuadContent, QuadRenderItem, RenderColor,
        RenderItemId, RenderItemIdentity, RenderItemInstance, RenderMaterialDescriptor,
        RenderMaterialFrameState, RenderMaterialId, RenderMaterialKind, RenderMaterialParamBlock,
        RenderMaterialPipelineKey, RenderMaterialQueueKind, RenderMaterialShaderSource,
        RenderPassGraph, RenderPlan, RenderPlanItem, RenderProcessPlan, RenderRect,
        RenderSceneRole, RenderSourceId, RenderTargetId, RenderTargetKind, ShellRenderInput,
        SurfacePresentationRole, SurfacePresentationSnapshot, SurfacePresentationState,
        SurfaceRenderItem,
    };

    use crate::compositor_render::{RenderViewSnapshot, RenderViewState};
    use crate::material::{RenderMaterialRequest, RenderMaterialRequestQueue};

    use super::{
        PreparedSceneResources, RenderTargetAllocationPlan, SurfaceBufferAttachmentSnapshot,
        SurfaceTextureBridgePlan, build_prepared_gpu_resources_system,
        build_prepared_scene_resources_system, build_render_target_allocation_plan_system,
        build_surface_texture_bridge_plan_system,
    };

    fn identity(id: u64) -> RenderItemIdentity {
        RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
    }

    #[test]
    fn target_allocation_plan_tracks_output_sized_targets() {
        let mut world = World::default();
        world.insert_resource(RenderViewSnapshot {
            views: vec![RenderViewState {
                output_id: OutputId(7),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1,
            }],
        });
        world.insert_resource(RenderPassGraph {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputExecutionPlan {
                    targets: BTreeMap::from([
                        (RenderTargetId(1), RenderTargetKind::OutputSwapchain(OutputId(7))),
                        (RenderTargetId(2), RenderTargetKind::OffscreenColor),
                    ]),
                    ..Default::default()
                },
            )]),
        });
        world.init_resource::<RenderTargetAllocationPlan>();

        let mut system = IntoSystem::into_system(build_render_target_allocation_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let plan = world.resource::<RenderTargetAllocationPlan>();
        assert_eq!(plan.outputs[&OutputId(7)].targets[&RenderTargetId(2)].width, 1920);
        assert_eq!(plan.outputs[&OutputId(7)].targets[&RenderTargetId(2)].height, 1080);
    }

    #[test]
    fn surface_texture_bridge_plan_tracks_visible_surface_imports() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(11),
                    surface_id: 11,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                })]),
            )]),
        });
        world.insert_resource(ShellRenderInput {
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: BTreeMap::from([(
                    11,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(OutputId(7)),
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: 100,
                            height: 100,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                )]),
            },
            ..Default::default()
        });
        world.insert_resource(nekoland_ecs::resources::SurfaceContentVersionSnapshot {
            versions: BTreeMap::from([(11, 3)]),
        });
        world.insert_resource(SurfaceBufferAttachmentSnapshot {
            surfaces: BTreeMap::from([(
                11,
                super::SurfaceBufferAttachmentState { attached: true, scale: 2 },
            )]),
        });
        world.insert_resource(PlatformSurfaceSnapshotState {
            surfaces: BTreeMap::from([(
                11,
                PlatformSurfaceSnapshot {
                    surface_id: 11,
                    kind: PlatformSurfaceKind::Toplevel,
                    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                    dmabuf_format: None,
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                    attached: true,
                    scale: 2,
                    content_version: 3,
                },
            )]),
        });
        world.insert_resource(ShellRenderInput {
            surface_presentation: nekoland_ecs::resources::SurfacePresentationSnapshot {
                surfaces: BTreeMap::from([(
                    11,
                    nekoland_ecs::resources::SurfacePresentationState {
                        visible: true,
                        target_output: Some(OutputId(2)),
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: 50,
                            height: 50,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: nekoland_ecs::resources::SurfacePresentationRole::Window,
                    },
                )]),
            },
            ..Default::default()
        });
        world.init_resource::<SurfaceTextureBridgePlan>();

        let mut system = IntoSystem::into_system(build_surface_texture_bridge_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let bridge = world.resource::<SurfaceTextureBridgePlan>();
        let descriptor = &bridge.surfaces[&11];
        assert!(descriptor.attached);
        assert_eq!(descriptor.surface_kind, PlatformSurfaceKind::Toplevel);
        assert_eq!(
            descriptor.buffer_source,
            nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm
        );
        assert_eq!(descriptor.scale, 2);
        assert_eq!(descriptor.content_version, 3);
        assert!(descriptor.target_outputs.contains(&OutputId(7)));
    }

    #[test]
    fn dmabuf_surface_imports_follow_platform_owned_import_strategy() {
        let mut world = World::default();
        world.insert_resource(SurfaceTextureBridgePlan {
            surfaces: BTreeMap::from([(
                11,
                nekoland_ecs::resources::SurfaceTextureImportDescriptor {
                    surface_id: 11,
                    surface_kind: PlatformSurfaceKind::Toplevel,
                    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::DmaBuf,
                    dmabuf_format: Some(nekoland_ecs::resources::PlatformDmabufFormat {
                        code: 875713112,
                        modifier: 0,
                    }),
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::Unsupported,
                    target_outputs: BTreeSet::from([OutputId(7)]),
                    content_version: 1,
                    attached: true,
                    scale: 1,
                },
            )]),
        });
        world.insert_resource(RenderTargetAllocationPlan::default());
        world.insert_resource(RenderMaterialRequestQueue::default());
        world.insert_resource(RenderMaterialFrameState::default());
        world.insert_resource(RenderProcessPlan::default());
        world.init_resource::<PreparedGpuResources>();

        let mut system = IntoSystem::into_system(build_prepared_gpu_resources_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let prepared = world.resource::<PreparedGpuResources>();
        assert_eq!(
            prepared.surface_imports[&11].strategy,
            nekoland_ecs::resources::PreparedSurfaceImportStrategy::Unsupported
        );
        let _ = prepared;

        world
            .resource_mut::<SurfaceTextureBridgePlan>()
            .surfaces
            .get_mut(&11)
            .unwrap()
            .import_strategy = nekoland_ecs::resources::PlatformSurfaceImportStrategy::DmaBufImport;
        let _ = system.run((), &mut world);

        let prepared = world.resource::<PreparedGpuResources>();
        assert_eq!(
            prepared.surface_imports[&11].strategy,
            nekoland_ecs::resources::PreparedSurfaceImportStrategy::DmaBufImport
        );

        world
            .resource_mut::<SurfaceTextureBridgePlan>()
            .surfaces
            .get_mut(&11)
            .unwrap()
            .import_strategy =
            nekoland_ecs::resources::PlatformSurfaceImportStrategy::ExternalTextureImport;
        let _ = system.run((), &mut world);

        let prepared = world.resource::<PreparedGpuResources>();
        assert_eq!(
            prepared.surface_imports[&11].strategy,
            nekoland_ecs::resources::PreparedSurfaceImportStrategy::ExternalTextureImport
        );
    }

    #[test]
    fn surface_texture_bridge_plan_carries_platform_dmabuf_metadata() {
        let mut world = World::default();
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(11),
                    surface_id: 11,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 100, height: 100 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                })]),
            )]),
        });
        world.insert_resource(nekoland_ecs::resources::SurfaceContentVersionSnapshot {
            versions: BTreeMap::from([(11, 5)]),
        });
        world.insert_resource(SurfaceBufferAttachmentSnapshot {
            surfaces: BTreeMap::from([(
                11,
                super::SurfaceBufferAttachmentState { attached: true, scale: 1 },
            )]),
        });
        world.insert_resource(PlatformSurfaceSnapshotState {
            surfaces: BTreeMap::from([(
                11,
                PlatformSurfaceSnapshot {
                    surface_id: 11,
                    kind: PlatformSurfaceKind::Toplevel,
                    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::DmaBuf,
                    dmabuf_format: Some(nekoland_ecs::resources::PlatformDmabufFormat {
                        code: 875713112,
                        modifier: 0,
                    }),
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::ExternalTextureImport,
                    attached: true,
                    scale: 1,
                    content_version: 5,
                },
            )]),
        });
        world.insert_resource(ShellRenderInput {
            surface_presentation: nekoland_ecs::resources::SurfacePresentationSnapshot {
                surfaces: BTreeMap::from([(
                    11,
                    nekoland_ecs::resources::SurfacePresentationState {
                        visible: true,
                        target_output: Some(OutputId(7)),
                        geometry: nekoland_ecs::components::SurfaceGeometry {
                            x: 0,
                            y: 0,
                            width: 100,
                            height: 100,
                        },
                        input_enabled: true,
                        damage_enabled: true,
                        role: nekoland_ecs::resources::SurfacePresentationRole::Window,
                    },
                )]),
            },
            ..Default::default()
        });
        world.init_resource::<SurfaceTextureBridgePlan>();

        let mut system = IntoSystem::into_system(build_surface_texture_bridge_plan_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let bridge = world.resource::<SurfaceTextureBridgePlan>();
        let descriptor = &bridge.surfaces[&11];
        assert_eq!(
            descriptor.dmabuf_format,
            Some(nekoland_ecs::resources::PlatformDmabufFormat { code: 875713112, modifier: 0 })
        );
        assert_eq!(
            descriptor.import_strategy,
            nekoland_ecs::resources::PlatformSurfaceImportStrategy::ExternalTextureImport
        );
    }

    #[test]
    fn prepared_scene_resources_classify_scene_items() {
        let mut world = World::default();
        world.insert_resource(RenderViewSnapshot {
            views: vec![RenderViewState {
                output_id: OutputId(7),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 2,
            }],
        });
        world.insert_resource(RenderPlan {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputRenderPlan::from_items([
                    RenderPlanItem::Surface(SurfaceRenderItem {
                        identity: identity(11),
                        surface_id: 11,
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 10, y: 20, width: 100, height: 120 },
                            opacity: 0.5,
                            clip_rect: Some(RenderRect { x: 20, y: 30, width: 40, height: 50 }),
                            z_index: 0,
                            scene_role: RenderSceneRole::Desktop,
                        },
                    }),
                    RenderPlanItem::Quad(QuadRenderItem {
                        identity: identity(12),
                        content: QuadContent::SolidColor {
                            color: RenderColor { r: 1, g: 2, b: 3, a: 255 },
                        },
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 8, height: 9 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 1,
                            scene_role: RenderSceneRole::Overlay,
                        },
                    }),
                    RenderPlanItem::Cursor(nekoland_ecs::resources::CursorRenderItem {
                        identity: identity(13),
                        source: CursorRenderSource::Named { icon_name: "default".to_owned() },
                        instance: RenderItemInstance {
                            rect: RenderRect { x: 33, y: 44, width: 10, height: 12 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: i32::MAX,
                            scene_role: RenderSceneRole::Cursor,
                        },
                    }),
                ]),
            )]),
        });
        world.insert_resource(SurfaceTextureBridgePlan {
            surfaces: BTreeMap::from([(
                11,
                super::SurfaceTextureImportDescriptor {
                    surface_id: 11,
                    surface_kind: PlatformSurfaceKind::Toplevel,
                    buffer_source: nekoland_ecs::resources::PlatformSurfaceBufferSource::Shm,
                    dmabuf_format: None,
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                    target_outputs: BTreeSet::from([OutputId(7)]),
                    content_version: 4,
                    attached: true,
                    scale: 2,
                },
            )]),
        });
        world.init_resource::<PreparedSceneResources>();

        let mut system = IntoSystem::into_system(build_prepared_scene_resources_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let prepared = world.resource::<PreparedSceneResources>();
        let output = &prepared.outputs[&OutputId(7)];
        assert_eq!(
            output.ordered_items,
            vec![RenderItemId(11), RenderItemId(12), RenderItemId(13)]
        );
        assert!(matches!(output.items[&RenderItemId(11)], PreparedSceneItem::Surface(_)));
        assert!(matches!(output.items[&RenderItemId(12)], PreparedSceneItem::Quad(_)));
        assert!(matches!(output.items[&RenderItemId(13)], PreparedSceneItem::CursorNamed(_)));
    }

    #[test]
    fn prepared_gpu_resources_track_targets_imports_and_material_bindings() {
        let mut world = World::default();
        world.insert_resource(RenderTargetAllocationPlan {
            outputs: BTreeMap::from([(
                OutputId(7),
                super::OutputTargetAllocationPlan {
                    targets: BTreeMap::from([(
                        RenderTargetId(2),
                        super::RenderTargetAllocationSpec {
                            kind: RenderTargetKind::OffscreenColor,
                            width: 1920,
                            height: 1080,
                        },
                    )]),
                },
            )]),
        });
        world.insert_resource(SurfaceTextureBridgePlan {
            surfaces: BTreeMap::from([(
                11,
                super::SurfaceTextureImportDescriptor {
                    surface_id: 11,
                    surface_kind: PlatformSurfaceKind::Toplevel,
                    buffer_source: PlatformSurfaceBufferSource::Shm,
                    dmabuf_format: None,
                    import_strategy:
                        nekoland_ecs::resources::PlatformSurfaceImportStrategy::ShmUpload,
                    target_outputs: BTreeSet::from([OutputId(7)]),
                    content_version: 3,
                    attached: true,
                    scale: 1,
                },
            )]),
        });
        world.insert_resource(RenderMaterialFrameState {
            descriptors: BTreeMap::from([(
                RenderMaterialId(5),
                RenderMaterialDescriptor {
                    debug_name: "blur".to_owned(),
                    pipeline_key: RenderMaterialPipelineKey::post_process(RenderMaterialKind::Blur),
                    shader_source: RenderMaterialShaderSource::Blur,
                    bind_group_layout:
                        nekoland_ecs::resources::RenderBindGroupLayoutKey::BlurUniforms,
                    queue_kind: RenderMaterialQueueKind::PostProcess,
                },
            )]),
            params: BTreeMap::from([(MaterialParamsId(3), RenderMaterialParamBlock::blur(6.0))]),
        });
        world.insert_resource(RenderMaterialRequestQueue {
            outputs: BTreeMap::from([(
                OutputId(7),
                vec![RenderMaterialRequest {
                    scene_role: RenderSceneRole::Compositor,
                    material_id: RenderMaterialId(5),
                    params_id: Some(MaterialParamsId(3)),
                    process_regions: Vec::new(),
                }],
            )]),
        });
        world.insert_resource(RenderProcessPlan {
            outputs: BTreeMap::from([(
                OutputId(7),
                OutputProcessPlan {
                    units: BTreeMap::from([(
                        ProcessUnitId(1),
                        ProcessUnit {
                            shader_key: ProcessShaderKey::Material(
                                RenderMaterialPipelineKey::post_process(RenderMaterialKind::Blur),
                            ),
                            input: ProcessInputRef {
                                target_id: RenderTargetId(2),
                                sample_rect: None,
                            },
                            output: ProcessTargetRef {
                                target_id: RenderTargetId(2),
                                output_rect: None,
                            },
                            uniforms: ProcessUniformBlock::default(),
                            process_regions: Vec::new(),
                        },
                    )]),
                    ordered_units: vec![ProcessUnitId(1)],
                    pass_units: BTreeMap::default(),
                },
            )]),
        });
        world.init_resource::<PreparedGpuResources>();

        let mut system = IntoSystem::into_system(build_prepared_gpu_resources_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let prepared = world.resource::<PreparedGpuResources>();
        let output = &prepared.outputs[&OutputId(7)];
        let binding_key = nekoland_ecs::resources::PreparedMaterialBindingKey {
            material_id: RenderMaterialId(5),
            params_id: Some(MaterialParamsId(3)),
        };

        assert_eq!(output.targets.len(), 1);
        assert_eq!(
            output.surface_imports.get(&11).map(|prepared_import| prepared_import.strategy),
            Some(nekoland_ecs::resources::PreparedSurfaceImportStrategy::ShmUpload)
        );
        assert_eq!(
            output
                .surface_imports
                .get(&11)
                .map(|prepared_import| prepared_import.cache_key.content_version),
            Some(3)
        );
        assert!(output.material_bindings.contains(&binding_key));
        assert_eq!(
            output.targets.get(&RenderTargetId(2)).map(|target| target.cache_key.output_id),
            Some(OutputId(7))
        );
        assert_eq!(
            prepared.surface_imports[&11].strategy,
            nekoland_ecs::resources::PreparedSurfaceImportStrategy::ShmUpload
        );
        assert_eq!(prepared.surface_imports[&11].cache_key.surface_id, 11);
        assert!(output.process_shaders.contains(&ProcessShaderKey::Material(
            RenderMaterialPipelineKey::post_process(RenderMaterialKind::Blur),
        )));
        assert_eq!(
            prepared.material_bindings[&binding_key]
                .params
                .as_ref()
                .and_then(nekoland_ecs::resources::RenderMaterialParamBlock::radius),
            Some(6.0)
        );
        assert_eq!(prepared.material_bindings[&binding_key].cache_key.output_id, OutputId(7));
    }
}
