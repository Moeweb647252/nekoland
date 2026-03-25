use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::{Query, Res, ResMut, Resource, With};
use bevy_ecs::world::World;
use nekoland_ecs::components::{LayerShellSurface, PopupSurface, XdgWindow};
use nekoland_ecs::resources::{
    CompositorSceneState, RenderRect, ShellRenderInput, SurfacePresentationRole,
    SurfacePresentationSnapshot, WaylandIngress,
};

use crate::animation::{
    AnimationBindingKey, AnimationProperty, AnimationTimelineStore, AnimationValue,
};
use crate::scene_source::{RenderInstanceKey, RenderSourceKey};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppearanceState {
    pub opacity: f32,
}

impl Default for AppearanceState {
    fn default() -> Self {
        Self { opacity: 1.0 }
    }
}

#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct AppearanceSnapshot {
    pub sources: BTreeMap<RenderSourceKey, AppearanceState>,
    pub instances: BTreeMap<RenderInstanceKey, AppearanceState>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionState {
    pub rect_override: Option<RenderRect>,
    pub clip_rect_override: Option<RenderRect>,
}

impl ProjectionState {
    pub fn is_empty(&self) -> bool {
        self.rect_override.is_none() && self.clip_rect_override.is_none()
    }
}

#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionSnapshot {
    pub sources: BTreeMap<RenderSourceKey, ProjectionState>,
    pub instances: BTreeMap<RenderInstanceKey, ProjectionState>,
}

type SurfaceAnimationQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static nekoland_ecs::components::WlSurfaceHandle,
        &'static nekoland_ecs::components::WindowAnimation,
    ),
    With<XdgWindow>,
>;
type PopupAnimationQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static nekoland_ecs::components::WlSurfaceHandle,
        &'static nekoland_ecs::components::WindowAnimation,
    ),
    With<PopupSurface>,
>;
type LayerAnimationQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static nekoland_ecs::components::WlSurfaceHandle,
        &'static nekoland_ecs::components::WindowAnimation,
    ),
    With<LayerShellSurface>,
>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BindingSamples {
    rect: Option<RenderRect>,
    clip_rect: Option<RenderRect>,
}

pub fn clear_scene_process_snapshots_system(
    mut appearance: ResMut<'_, AppearanceSnapshot>,
    mut projection: ResMut<'_, ProjectionSnapshot>,
) {
    appearance.sources.clear();
    appearance.instances.clear();
    projection.sources.clear();
    projection.instances.clear();
}

pub fn surface_scene_process_snapshot_system(
    windows: SurfaceAnimationQuery<'_, '_>,
    popups: PopupAnimationQuery<'_, '_>,
    layers: LayerAnimationQuery<'_, '_>,
    wayland_ingress: Res<'_, WaylandIngress>,
    shell_render_input: Res<'_, ShellRenderInput>,
    timelines: Res<'_, AnimationTimelineStore>,
    mut appearance: ResMut<'_, AppearanceSnapshot>,
    mut projection: ResMut<'_, ProjectionSnapshot>,
) {
    let live_outputs = wayland_ingress
        .output_snapshots
        .outputs
        .iter()
        .map(|output| output.output_id)
        .collect::<Vec<_>>();
    let surface_presentation = &shell_render_input.surface_presentation;

    for (surface, animation) in windows.iter() {
        snapshot_surface_process_entry(
            surface.id,
            RenderSourceKey::window(surface.id),
            animation,
            SurfacePresentationRole::Window,
            &live_outputs,
            surface_presentation,
            &timelines,
            &mut appearance,
            &mut projection,
        );
    }

    for (surface, animation) in popups.iter() {
        snapshot_surface_process_entry(
            surface.id,
            RenderSourceKey::popup(surface.id),
            animation,
            SurfacePresentationRole::Popup,
            &live_outputs,
            surface_presentation,
            &timelines,
            &mut appearance,
            &mut projection,
        );
    }

    for (surface, animation) in layers.iter() {
        snapshot_surface_process_entry(
            surface.id,
            RenderSourceKey::layer(surface.id),
            animation,
            SurfacePresentationRole::Layer,
            &live_outputs,
            surface_presentation,
            &timelines,
            &mut appearance,
            &mut projection,
        );
    }
}

fn snapshot_surface_process_entry(
    surface_id: u64,
    source_key: RenderSourceKey,
    animation: &nekoland_ecs::components::WindowAnimation,
    expected_role: SurfacePresentationRole,
    live_outputs: &[nekoland_ecs::components::OutputId],
    surface_presentation: &SurfacePresentationSnapshot,
    timelines: &AnimationTimelineStore,
    appearance: &mut AppearanceSnapshot,
    projection: &mut ProjectionSnapshot,
) {
    let source_binding = AnimationBindingKey::Source(source_key.clone());
    let source_samples = binding_samples(timelines, &source_binding);

    let mut source_appearance = AppearanceState { opacity: opacity_for_animation(animation) };
    if let Some(opacity) = sampled_opacity(timelines, &source_binding) {
        source_appearance.opacity = opacity;
    }
    appearance.sources.insert(source_key.clone(), source_appearance);

    let source_projection = ProjectionState {
        rect_override: source_samples.rect,
        clip_rect_override: source_samples.clip_rect,
    };
    if !source_projection.is_empty() {
        projection.sources.insert(source_key.clone(), source_projection);
    }

    let target_outputs = surface_presentation
        .surfaces
        .get(&surface_id)
        .filter(|state| state.visible && state.role == expected_role)
        .map(|state| {
            if let Some(target_output) = state.target_output {
                vec![target_output]
            } else {
                live_outputs.to_vec()
            }
        })
        .unwrap_or_default();

    for output_id in target_outputs {
        let instance_key = RenderInstanceKey::new(source_key.clone(), output_id, 0);
        let instance_binding = AnimationBindingKey::Instance(instance_key.clone());
        if let Some(opacity) = sampled_opacity(timelines, &instance_binding) {
            appearance.instances.insert(instance_key.clone(), AppearanceState { opacity });
        }

        let instance_projection = projection_state_from_samples(timelines, &instance_binding);
        if !instance_projection.is_empty() {
            projection.instances.insert(instance_key, instance_projection);
        }
    }
}

pub fn compositor_scene_process_snapshot_system(
    compositor_scene: Option<Res<'_, CompositorSceneState>>,
    timelines: Res<'_, AnimationTimelineStore>,
    mut appearance: ResMut<'_, AppearanceSnapshot>,
    mut projection: ResMut<'_, ProjectionSnapshot>,
) {
    let Some(compositor_scene) = compositor_scene else {
        return;
    };

    for (output_id, output_scene) in &compositor_scene.outputs {
        for (entry_id, _) in output_scene.iter_ordered() {
            let source_key = RenderSourceKey::compositor(entry_id);
            let source_binding = AnimationBindingKey::Source(source_key.clone());
            if let Some(opacity) = sampled_opacity(&timelines, &source_binding) {
                appearance.sources.insert(source_key.clone(), AppearanceState { opacity });
            }

            let source_projection = projection_state_from_samples(&timelines, &source_binding);
            if !source_projection.is_empty() {
                projection.sources.insert(source_key.clone(), source_projection);
            }

            let instance_key = RenderInstanceKey::compositor(entry_id, *output_id);
            let instance_binding = AnimationBindingKey::Instance(instance_key.clone());
            if let Some(opacity) = sampled_opacity(&timelines, &instance_binding) {
                appearance.instances.insert(instance_key.clone(), AppearanceState { opacity });
            }

            let instance_projection = projection_state_from_samples(&timelines, &instance_binding);
            if !instance_projection.is_empty() {
                projection.instances.insert(instance_key, instance_projection);
            }
        }
    }
}

pub fn extract_scene_process_snapshots(main_world: &mut World, render_world: &mut World) {
    let mut appearance = AppearanceSnapshot::default();
    let mut projection = ProjectionSnapshot::default();

    let Some(timelines) = main_world.get_resource::<AnimationTimelineStore>().cloned() else {
        render_world.insert_resource(appearance);
        render_world.insert_resource(projection);
        return;
    };

    let live_outputs = main_world
        .resource::<WaylandIngress>()
        .output_snapshots
        .outputs
        .iter()
        .map(|output| output.output_id)
        .collect::<Vec<_>>();
    let shell_render_input = main_world.resource::<ShellRenderInput>().clone();
    let surface_presentation = &shell_render_input.surface_presentation;

    let mut windows = main_world.query_filtered::<(
        &'static nekoland_ecs::components::WlSurfaceHandle,
        &'static nekoland_ecs::components::WindowAnimation,
    ), With<XdgWindow>>();
    let mut popups = main_world.query_filtered::<(
        &'static nekoland_ecs::components::WlSurfaceHandle,
        &'static nekoland_ecs::components::WindowAnimation,
    ), With<PopupSurface>>();
    let mut layers = main_world.query_filtered::<(
        &'static nekoland_ecs::components::WlSurfaceHandle,
        &'static nekoland_ecs::components::WindowAnimation,
    ), With<LayerShellSurface>>();

    for (surface, animation) in windows.iter(main_world) {
        snapshot_surface_process_entry(
            surface.id,
            RenderSourceKey::window(surface.id),
            animation,
            SurfacePresentationRole::Window,
            &live_outputs,
            surface_presentation,
            &timelines,
            &mut appearance,
            &mut projection,
        );
    }

    for (surface, animation) in popups.iter(main_world) {
        snapshot_surface_process_entry(
            surface.id,
            RenderSourceKey::popup(surface.id),
            animation,
            SurfacePresentationRole::Popup,
            &live_outputs,
            surface_presentation,
            &timelines,
            &mut appearance,
            &mut projection,
        );
    }

    for (surface, animation) in layers.iter(main_world) {
        snapshot_surface_process_entry(
            surface.id,
            RenderSourceKey::layer(surface.id),
            animation,
            SurfacePresentationRole::Layer,
            &live_outputs,
            surface_presentation,
            &timelines,
            &mut appearance,
            &mut projection,
        );
    }

    if let Some(compositor_scene) = main_world.get_resource::<CompositorSceneState>() {
        for (output_id, output_scene) in &compositor_scene.outputs {
            for (entry_id, _) in output_scene.iter_ordered() {
                let source_key = RenderSourceKey::compositor(entry_id);
                let source_binding = AnimationBindingKey::Source(source_key.clone());
                if let Some(opacity) = sampled_opacity(&timelines, &source_binding) {
                    appearance.sources.insert(source_key.clone(), AppearanceState { opacity });
                }

                let source_projection = projection_state_from_samples(&timelines, &source_binding);
                if !source_projection.is_empty() {
                    projection.sources.insert(source_key.clone(), source_projection);
                }

                let instance_key = RenderInstanceKey::compositor(entry_id, *output_id);
                let instance_binding = AnimationBindingKey::Instance(instance_key.clone());
                if let Some(opacity) = sampled_opacity(&timelines, &instance_binding) {
                    appearance.instances.insert(instance_key.clone(), AppearanceState { opacity });
                }

                let instance_projection =
                    projection_state_from_samples(&timelines, &instance_binding);
                if !instance_projection.is_empty() {
                    projection.instances.insert(instance_key, instance_projection);
                }
            }
        }
    }

    render_world.insert_resource(appearance);
    render_world.insert_resource(projection);
}

pub fn prune_stale_compositor_animation_tracks_system(
    compositor_scene: Option<Res<'_, CompositorSceneState>>,
    mut timelines: ResMut<'_, AnimationTimelineStore>,
) {
    let mut live_bindings = BTreeSet::new();

    if let Some(compositor_scene) = compositor_scene {
        for (output_id, output_scene) in &compositor_scene.outputs {
            for (entry_id, _) in output_scene.iter_ordered() {
                let source_key = RenderSourceKey::compositor(entry_id);
                live_bindings.insert(AnimationBindingKey::Source(source_key.clone()));
                live_bindings.insert(AnimationBindingKey::Instance(RenderInstanceKey::compositor(
                    entry_id, *output_id,
                )));
            }
        }
    }

    timelines.retain_tracks(|binding, _, _| {
        if !is_compositor_binding(binding) {
            return true;
        }
        live_bindings.contains(binding)
    });
}

pub fn apply_appearance_snapshot(
    opacity: &mut f32,
    source_key: &RenderSourceKey,
    instance_key: &RenderInstanceKey,
    snapshot: Option<&AppearanceSnapshot>,
) {
    let Some(snapshot) = snapshot else {
        return;
    };

    if let Some(state) = snapshot.sources.get(source_key) {
        *opacity = state.opacity;
    }
    if let Some(state) = snapshot.instances.get(instance_key) {
        *opacity = state.opacity;
    }
}

pub fn apply_projection_snapshot(
    rect: &mut RenderRect,
    clip_rect: &mut Option<RenderRect>,
    source_key: &RenderSourceKey,
    instance_key: &RenderInstanceKey,
    snapshot: Option<&ProjectionSnapshot>,
) {
    let Some(snapshot) = snapshot else {
        return;
    };

    if let Some(state) = snapshot.sources.get(source_key) {
        if let Some(source_rect) = state.rect_override {
            *rect = source_rect;
        }
        if let Some(source_clip) = state.clip_rect_override {
            *clip_rect = Some(source_clip);
        }
    }
    if let Some(state) = snapshot.instances.get(instance_key) {
        if let Some(instance_rect) = state.rect_override {
            *rect = instance_rect;
        }
        if let Some(instance_clip) = state.clip_rect_override {
            *clip_rect = Some(instance_clip);
        }
    }
}

fn is_compositor_binding(binding: &AnimationBindingKey) -> bool {
    match binding {
        AnimationBindingKey::Source(key) => key.namespace == "compositor",
        AnimationBindingKey::Instance(key) => key.source_key.namespace == "compositor",
    }
}

fn projection_state_from_samples(
    timelines: &AnimationTimelineStore,
    binding: &AnimationBindingKey,
) -> ProjectionState {
    let samples = binding_samples(timelines, binding);
    ProjectionState { rect_override: samples.rect, clip_rect_override: samples.clip_rect }
}

fn binding_samples(
    timelines: &AnimationTimelineStore,
    binding: &AnimationBindingKey,
) -> BindingSamples {
    BindingSamples {
        rect: match timelines.sampled_value(binding, AnimationProperty::Rect) {
            Some(AnimationValue::Rect(rect)) => Some(*rect),
            _ => None,
        },
        clip_rect: match timelines.sampled_value(binding, AnimationProperty::ClipRect) {
            Some(AnimationValue::Rect(rect)) => Some(*rect),
            _ => None,
        },
    }
}

fn sampled_opacity(
    timelines: &AnimationTimelineStore,
    binding: &AnimationBindingKey,
) -> Option<f32> {
    match timelines.sampled_value(binding, AnimationProperty::Opacity) {
        Some(AnimationValue::Float(opacity)) => Some((*opacity).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn opacity_for_animation(animation: &nekoland_ecs::components::WindowAnimation) -> f32 {
    if animation.progress == 0.0 && animation.fade == nekoland_ecs::components::FadeState::Idle {
        1.0
    } else {
        animation.progress.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::PreRenderSchedule;
    use nekoland_ecs::bundles::WindowBundle;
    use nekoland_ecs::components::{OutputId, WlSurfaceHandle, XdgWindow};
    use nekoland_ecs::resources::{
        CompositorClock, CompositorSceneEntry, CompositorSceneEntryId, CompositorSceneState,
        OutputCompositorScene, OutputGeometrySnapshot, RenderItemInstance, RenderRect,
        RenderSceneRole, ShellRenderInput, SurfacePresentationRole, SurfacePresentationState,
        WaylandIngress,
    };

    use crate::animation::{
        AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
        AnimationTrack, AnimationValue, advance_animation_timelines_system,
    };
    use crate::scene_source::{RenderInstanceKey, RenderSourceKey};

    use super::{
        AppearanceSnapshot, ProjectionSnapshot, clear_scene_process_snapshots_system,
        compositor_scene_process_snapshot_system, prune_stale_compositor_animation_tracks_system,
        surface_scene_process_snapshot_system,
    };

    #[test]
    fn surface_process_snapshots_capture_source_and_instance_samples() {
        let mut app = NekolandApp::new("surface-process-snapshot-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 50 })
            .init_resource::<AnimationTimelineStore>()
            .init_resource::<AppearanceSnapshot>()
            .init_resource::<ProjectionSnapshot>()
            .insert_resource(WaylandIngress {
                output_snapshots: nekoland_ecs::resources::OutputSnapshotState {
                    outputs: vec![OutputGeometrySnapshot {
                        output_id: OutputId(3),
                        name: "Virtual-1".to_owned(),
                        x: 0,
                        y: 0,
                        width: 1280,
                        height: 720,
                        scale: 1,
                        refresh_millihz: 60_000,
                    }],
                },
                ..WaylandIngress::default()
            })
            .init_resource::<ShellRenderInput>()
            .add_systems(
                PreRenderSchedule,
                (
                    advance_animation_timelines_system,
                    clear_scene_process_snapshots_system,
                    surface_scene_process_snapshot_system,
                )
                    .chain(),
            );

        let output_id = OutputId(3);
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 13 },
            window: XdgWindow::default(),
            ..Default::default()
        });
        app.inner_mut()
            .world_mut()
            .resource_mut::<ShellRenderInput>()
            .surface_presentation
            .surfaces
            .insert(
                13,
                SurfacePresentationState {
                    geometry: nekoland_ecs::components::SurfaceGeometry {
                        x: 0,
                        y: 0,
                        width: 50,
                        height: 50,
                    },
                    role: SurfacePresentationRole::Window,
                    target_output: Some(output_id),
                    visible: true,
                    input_enabled: true,
                    damage_enabled: true,
                },
            );
        {
            let mut timelines =
                app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>();
            let source_binding = AnimationBindingKey::Source(RenderSourceKey::window(13));
            timelines.upsert_track(
                source_binding,
                AnimationTrack {
                    property: AnimationProperty::Opacity,
                    from: AnimationValue::Float(0.0),
                    to: AnimationValue::Float(1.0),
                    start_uptime_millis: 0,
                    duration_millis: 100,
                    easing: AnimationEasing::Linear,
                },
            );
            let instance_binding = AnimationBindingKey::Instance(RenderInstanceKey::new(
                RenderSourceKey::window(13),
                output_id,
                0,
            ));
            timelines.upsert_track(
                instance_binding,
                AnimationTrack {
                    property: AnimationProperty::Rect,
                    from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 50, height: 50 }),
                    to: AnimationValue::Rect(RenderRect { x: 10, y: 20, width: 60, height: 70 }),
                    start_uptime_millis: 0,
                    duration_millis: 100,
                    easing: AnimationEasing::Linear,
                },
            );
        }

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let appearance = app.inner().world().resource::<AppearanceSnapshot>();
        let projection = app.inner().world().resource::<ProjectionSnapshot>();
        assert_eq!(
            appearance.sources.get(&RenderSourceKey::window(13)).map(|state| state.opacity),
            Some(0.5)
        );
        assert_eq!(
            projection
                .instances
                .get(&RenderInstanceKey::new(RenderSourceKey::window(13), output_id, 0))
                .and_then(|state| state.rect_override),
            Some(RenderRect { x: 5, y: 10, width: 55, height: 60 })
        );
    }

    #[test]
    fn stale_compositor_tracks_are_pruned() {
        let mut app = NekolandApp::new("stale-compositor-track-prune-test");
        app.inner_mut()
            .init_resource::<AnimationTimelineStore>()
            .init_resource::<CompositorSceneState>()
            .add_systems(PreRenderSchedule, prune_stale_compositor_animation_tracks_system);

        let source_binding =
            AnimationBindingKey::Source(RenderSourceKey::compositor(CompositorSceneEntryId(7)));
        app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>().upsert_track(
            source_binding.clone(),
            AnimationTrack {
                property: AnimationProperty::Opacity,
                from: AnimationValue::Float(0.0),
                to: AnimationValue::Float(1.0),
                start_uptime_millis: 0,
                duration_millis: 100,
                easing: AnimationEasing::Linear,
            },
        );

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);
        assert!(
            app.inner()
                .world()
                .resource::<AnimationTimelineStore>()
                .sampled_value(&source_binding, AnimationProperty::Opacity)
                .is_none()
        );
    }

    #[test]
    fn compositor_process_snapshots_capture_instance_samples() {
        let mut app = NekolandApp::new("compositor-process-snapshot-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 100 })
            .init_resource::<AnimationTimelineStore>()
            .init_resource::<AppearanceSnapshot>()
            .init_resource::<ProjectionSnapshot>()
            .insert_resource(CompositorSceneState {
                outputs: BTreeMap::from([(
                    OutputId(3),
                    OutputCompositorScene::from_entries([(
                        CompositorSceneEntryId(9),
                        CompositorSceneEntry::backdrop(RenderItemInstance {
                            rect: RenderRect { x: 0, y: 0, width: 10, height: 10 },
                            opacity: 1.0,
                            clip_rect: None,
                            z_index: 4,
                            scene_role: RenderSceneRole::Compositor,
                        }),
                    )]),
                )]),
            })
            .add_systems(
                PreRenderSchedule,
                (
                    advance_animation_timelines_system,
                    clear_scene_process_snapshots_system,
                    compositor_scene_process_snapshot_system,
                )
                    .chain(),
            );
        app.inner_mut().world_mut().resource_mut::<AnimationTimelineStore>().upsert_track(
            AnimationBindingKey::Instance(RenderInstanceKey::compositor(
                CompositorSceneEntryId(9),
                OutputId(3),
            )),
            AnimationTrack {
                property: AnimationProperty::ClipRect,
                from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 10, height: 10 }),
                to: AnimationValue::Rect(RenderRect { x: 1, y: 2, width: 5, height: 6 }),
                start_uptime_millis: 0,
                duration_millis: 100,
                easing: AnimationEasing::Linear,
            },
        );

        app.inner_mut().world_mut().run_schedule(PreRenderSchedule);

        let projection = app.inner().world().resource::<ProjectionSnapshot>();
        assert_eq!(
            projection
                .instances
                .get(&RenderInstanceKey::compositor(CompositorSceneEntryId(9), OutputId(3)))
                .and_then(|state| state.clip_rect_override),
            Some(RenderRect { x: 1, y: 2, width: 5, height: 6 })
        );
    }
}
