use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};

use bevy_ecs::prelude::{Local, Query, Res, ResMut};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::components::{OutputId, SurfaceContentVersion, WlSurfaceHandle};
use nekoland_ecs::resources::{
    DamageRect, DamageState, OutputDamageRegions, RenderItemId, RenderPassGraph, RenderPassKind,
    RenderPlan, RenderPlanItem, RenderRect, SurfacePresentationSnapshot,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DamageTracker;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrackedSceneDamage {
    rect: DamageRect,
    render_signature: u64,
}

type OutputDamageSnapshot = BTreeMap<OutputId, BTreeMap<RenderItemId, TrackedSceneDamage>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DamageTrackerState {
    previous_snapshot: Option<OutputDamageSnapshot>,
}

type DamageSurfaceVersions<'w, 's> =
    Query<'w, 's, (&'static WlSurfaceHandle, &'static SurfaceContentVersion)>;
type DamageOutputs<'w, 's> = Query<'w, 's, &'static OutputId>;

#[derive(SystemParam)]
pub(crate) struct DamageTrackingParams<'w, 's> {
    surface_versions: DamageSurfaceVersions<'w, 's>,
    outputs: DamageOutputs<'w, 's>,
    render_plan: Res<'w, RenderPlan>,
    render_graph: Res<'w, RenderPassGraph>,
    surface_presentation: Option<Res<'w, SurfacePresentationSnapshot>>,
    damage_state: ResMut<'w, DamageState>,
    output_damage_regions: ResMut<'w, OutputDamageRegions>,
    tracker_state: Local<'s, DamageTrackerState>,
}

/// Derives per-output scene damage from changes in the visible compositor scene graph.
///
/// The tracker keeps the previous output-local geometry plus a content-commit version for each
/// visible surface. Geometry-only changes emit a symmetric difference; content commits emit the
/// full current rect, and disappearing geometry emits the previous rect.
pub(crate) fn damage_tracking_system(damage: DamageTrackingParams<'_, '_>) {
    let DamageTrackingParams {
        surface_versions,
        outputs,
        render_plan,
        render_graph,
        surface_presentation,
        mut damage_state,
        mut output_damage_regions,
        mut tracker_state,
    } = damage;

    let live_output_ids = outputs.iter().copied().collect::<BTreeSet<_>>();
    let mut current_snapshot = live_output_ids
        .iter()
        .copied()
        .map(|output_id| (output_id, BTreeMap::new()))
        .collect::<OutputDamageSnapshot>();
    let surface_versions = surface_versions
        .iter()
        .map(|(surface, content_version)| (surface.id, content_version.value))
        .collect::<HashMap<_, _>>();
    let surface_presentation = surface_presentation.as_deref();

    for output_id in &live_output_ids {
        let Some(output_plan) = render_plan.outputs.get(output_id) else {
            continue;
        };
        let Some(execution) = render_graph.outputs.get(output_id) else {
            continue;
        };

        for pass_id in execution.reachable_passes_in_order() {
            let Some(pass) = execution.passes.get(&pass_id) else {
                continue;
            };
            if pass.kind != RenderPassKind::Scene {
                continue;
            }

            for item_id in pass.item_ids() {
                let Some(item) = output_plan.item(*item_id) else {
                    continue;
                };
                if surface_damage_disabled(item, surface_presentation) {
                    continue;
                }
                let Some(rect) = item.instance().visible_rect().map(damage_rect_from_render_rect)
                else {
                    continue;
                };

                current_snapshot.entry(*output_id).or_default().insert(
                    *item_id,
                    TrackedSceneDamage {
                        rect,
                        render_signature: render_signature_for_item(item, &surface_versions),
                    },
                );
            }
        }
    }

    let damage_regions = tracker_state
        .previous_snapshot
        .as_ref()
        .map(|previous_snapshot| diff_damage_snapshots(previous_snapshot, &current_snapshot))
        .unwrap_or_else(|| {
            current_snapshot
                .iter()
                .map(|(output_id, surfaces)| {
                    (
                        *output_id,
                        normalize_damage_rects(
                            surfaces.values().map(|surface| surface.rect.clone()).collect(),
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        });
    let count = damage_regions.values().map(Vec::len).sum::<usize>();
    damage_state.full_redraw = tracker_state.previous_snapshot.as_ref() != Some(&current_snapshot);
    tracker_state.previous_snapshot = Some(current_snapshot);
    output_damage_regions.regions = damage_regions;

    tracing::trace!(count, full_redraw = damage_state.full_redraw, "damage tracking tick");
}

fn surface_damage_disabled(
    item: &RenderPlanItem,
    surface_presentation: Option<&SurfacePresentationSnapshot>,
) -> bool {
    let Some(surface_id) = item.surface_id() else {
        return false;
    };

    surface_presentation
        .and_then(|snapshot| snapshot.surfaces.get(&surface_id))
        .is_some_and(|state| !state.damage_enabled)
}

fn damage_rect_from_render_rect(rect: RenderRect) -> DamageRect {
    DamageRect { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
}

fn render_signature_for_item(item: &RenderPlanItem, surface_versions: &HashMap<u64, u64>) -> u64 {
    let mut hasher = DefaultHasher::new();

    match item {
        RenderPlanItem::Surface(item) => {
            0_u8.hash(&mut hasher);
            item.surface_id.hash(&mut hasher);
            surface_versions.get(&item.surface_id).copied().unwrap_or_default().hash(&mut hasher);
            item.instance.opacity.to_bits().hash(&mut hasher);
        }
        RenderPlanItem::SolidRect(item) => {
            1_u8.hash(&mut hasher);
            item.color.r.hash(&mut hasher);
            item.color.g.hash(&mut hasher);
            item.color.b.hash(&mut hasher);
            item.color.a.hash(&mut hasher);
            item.instance.opacity.to_bits().hash(&mut hasher);
        }
        RenderPlanItem::Backdrop(_) => {
            2_u8.hash(&mut hasher);
            item.instance().opacity.to_bits().hash(&mut hasher);
        }
    }

    hasher.finish()
}

fn diff_damage_snapshots(
    previous_snapshot: &OutputDamageSnapshot,
    current_snapshot: &OutputDamageSnapshot,
) -> BTreeMap<OutputId, Vec<DamageRect>> {
    current_snapshot
        .iter()
        .map(|(output_id, current_surfaces)| {
            let previous_surfaces = previous_snapshot.get(output_id);
            let surface_ids = current_surfaces
                .keys()
                .chain(previous_surfaces.into_iter().flat_map(|surfaces| surfaces.keys()))
                .copied()
                .collect::<BTreeSet<_>>();
            let mut damage = Vec::new();

            for surface_id in surface_ids {
                let current = current_surfaces.get(&surface_id);
                let previous = previous_surfaces.and_then(|surfaces| surfaces.get(&surface_id));
                match (previous, current) {
                    (Some(previous), Some(current)) if previous != current => {
                        if previous.render_signature != current.render_signature {
                            damage.push(previous.rect.clone());
                            damage.push(current.rect.clone());
                        } else {
                            damage.extend(symmetric_difference(&previous.rect, &current.rect));
                        }
                    }
                    (None, Some(current)) => damage.push(current.rect.clone()),
                    (Some(previous), None) => damage.push(previous.rect.clone()),
                    _ => {}
                }
            }

            (*output_id, normalize_damage_rects(damage))
        })
        .collect()
}

fn symmetric_difference(left: &DamageRect, right: &DamageRect) -> Vec<DamageRect> {
    let Some(intersection) = intersect_damage_rects(left, right) else {
        return normalize_damage_rects(vec![left.clone(), right.clone()]);
    };

    let mut diff = subtract_damage_rect(left, &intersection);
    diff.extend(subtract_damage_rect(right, &intersection));
    normalize_damage_rects(diff)
}

fn intersect_damage_rects(left: &DamageRect, right: &DamageRect) -> Option<DamageRect> {
    let left_x2 = i64::from(left.x) + i64::from(left.width);
    let left_y2 = i64::from(left.y) + i64::from(left.height);
    let right_x2 = i64::from(right.x) + i64::from(right.width);
    let right_y2 = i64::from(right.y) + i64::from(right.height);

    let x1 = i64::from(left.x).max(i64::from(right.x));
    let y1 = i64::from(left.y).max(i64::from(right.y));
    let x2 = left_x2.min(right_x2);
    let y2 = left_y2.min(right_y2);

    (x1 < x2 && y1 < y2).then(|| DamageRect {
        x: x1.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y: y1.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width: (x2 - x1).min(i64::from(u32::MAX)) as u32,
        height: (y2 - y1).min(i64::from(u32::MAX)) as u32,
    })
}

fn subtract_damage_rect(rect: &DamageRect, intersection: &DamageRect) -> Vec<DamageRect> {
    let rect_x1 = i64::from(rect.x);
    let rect_y1 = i64::from(rect.y);
    let rect_x2 = rect_x1 + i64::from(rect.width);
    let rect_y2 = rect_y1 + i64::from(rect.height);
    let inter_x1 = i64::from(intersection.x);
    let inter_y1 = i64::from(intersection.y);
    let inter_x2 = inter_x1 + i64::from(intersection.width);
    let inter_y2 = inter_y1 + i64::from(intersection.height);
    let mut pieces = Vec::new();

    push_piece(&mut pieces, rect_x1, rect_y1, rect_x2, inter_y1);
    push_piece(&mut pieces, rect_x1, inter_y2, rect_x2, rect_y2);
    push_piece(&mut pieces, rect_x1, inter_y1, inter_x1, inter_y2);
    push_piece(&mut pieces, inter_x2, inter_y1, rect_x2, inter_y2);

    pieces
}

fn push_piece(pieces: &mut Vec<DamageRect>, x1: i64, y1: i64, x2: i64, y2: i64) {
    if x1 >= x2 || y1 >= y2 {
        return;
    }

    pieces.push(DamageRect {
        x: x1.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y: y1.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width: (x2 - x1).min(i64::from(u32::MAX)) as u32,
        height: (y2 - y1).min(i64::from(u32::MAX)) as u32,
    });
}

fn normalize_damage_rects(rects: Vec<DamageRect>) -> Vec<DamageRect> {
    let mut normalized =
        rects.into_iter().filter(|rect| rect.width != 0 && rect.height != 0).collect::<Vec<_>>();
    normalized.sort_by_key(|rect| (rect.x, rect.y, rect.width, rect.height));
    normalized.dedup();
    normalized
}

#[cfg(test)]
mod tests {
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        BufferState, DesiredOutputName, LayerShellSurface, OutputDevice, OutputId, OutputKind,
        OutputProperties, SurfaceContentVersion, SurfaceGeometry, WindowViewportVisibility,
        WlSurfaceHandle, XdgPopup, XdgWindow,
    };
    use nekoland_ecs::resources::{
        DamageState, OutputDamageRegions, RenderColor, RenderItemInstance, RenderPassGraph,
        RenderPlan, RenderRect, RenderSceneRole, SurfacePresentationRole,
        SurfacePresentationSnapshot, SurfacePresentationState, WindowStackingState,
    };

    use crate::{
        compositor_render::{assemble_render_plan_system, emit_desktop_scene_contributions_system},
        material::{RenderMaterialParamsStore, RenderMaterialRegistry, RenderMaterialRequestQueue},
        render_graph::build_render_graph_system,
        scene_source::{
            ExternalSceneContributionState, RenderInstanceKey, RenderSceneContribution,
            RenderSceneContributionPayload, RenderSceneContributionQueue,
            RenderSceneIdentityRegistry, RenderSourceKey, clear_scene_contributions_system,
        },
    };

    use super::damage_tracking_system;

    fn output_id_by_name(world: &mut bevy_ecs::world::World, name: &str) -> OutputId {
        world
            .query::<(&OutputId, &OutputDevice)>()
            .iter(world)
            .find(|(_, output)| output.name == name)
            .map(|(output_id, _)| *output_id)
            .unwrap_or_else(|| panic!("missing output id for {name}"))
    }

    fn install_damage_pipeline(app: &mut NekolandApp) {
        app.inner_mut()
            .init_resource::<RenderPlan>()
            .init_resource::<RenderPassGraph>()
            .init_resource::<RenderMaterialRegistry>()
            .init_resource::<RenderMaterialParamsStore>()
            .init_resource::<RenderMaterialRequestQueue>()
            .init_resource::<RenderSceneContributionQueue>()
            .init_resource::<ExternalSceneContributionState>()
            .init_resource::<RenderSceneIdentityRegistry>()
            .init_resource::<WindowStackingState>()
            .init_resource::<DamageState>()
            .init_resource::<OutputDamageRegions>()
            .add_systems(
                RenderSchedule,
                (
                    clear_scene_contributions_system,
                    emit_desktop_scene_contributions_system,
                    crate::scene_source::emit_external_scene_contributions_system,
                    assemble_render_plan_system,
                    build_render_graph_system,
                    damage_tracking_system,
                )
                    .chain(),
            );
    }

    #[test]
    fn damage_regions_are_scoped_per_output_and_recompute_on_output_change() {
        let mut app = NekolandApp::new("damage-tracker-output-routing-test");
        install_damage_pipeline(&mut app);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: "one".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "HDMI-A-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: "two".to_owned(),
            },
            properties: OutputProperties {
                width: 1920,
                height: 1080,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let virtual_id = output_id_by_name(app.inner_mut().world_mut(), "Virtual-1");
        let hdmi_id = output_id_by_name(app.inner_mut().world_mut(), "HDMI-A-1");

        let secondary_window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 2 },
                geometry: SurfaceGeometry { x: 20, y: 30, width: 120, height: 90 },
                viewport_visibility: WindowViewportVisibility {
                    visible: true,
                    output: Some(hdmi_id),
                },
                window: XdgWindow::default(),
                ..Default::default()
            })
            .id();
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 1 },
            geometry: SurfaceGeometry { x: 10, y: 15, width: 100, height: 80 },
            viewport_visibility: WindowViewportVisibility {
                visible: true,
                output: Some(virtual_id),
            },
            window: XdgWindow::default(),
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn((
            WlSurfaceHandle { id: 3 },
            XdgPopup::default(),
            BufferState { attached: true, scale: 1 },
            SurfaceGeometry { x: 5, y: 7, width: 30, height: 20 },
            bevy_ecs::hierarchy::ChildOf(secondary_window),
        ));
        app.inner_mut().world_mut().spawn((
            LayerShellSurface::default(),
            WlSurfaceHandle { id: 4 },
            SurfaceGeometry { x: 0, y: 0, width: 1280, height: 32 },
            BufferState { attached: true, scale: 1 },
            DesiredOutputName(Some("Virtual-1".to_owned())),
        ));
        app.inner_mut().world_mut().insert_resource(SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([
                (
                    1,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(virtual_id),
                        geometry: SurfaceGeometry { x: 10, y: 15, width: 100, height: 80 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                ),
                (
                    2,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(hdmi_id),
                        geometry: SurfaceGeometry { x: 20, y: 30, width: 120, height: 90 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                ),
                (
                    3,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(hdmi_id),
                        geometry: SurfaceGeometry { x: 5, y: 7, width: 30, height: 20 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Popup,
                    },
                ),
                (
                    4,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(virtual_id),
                        geometry: SurfaceGeometry { x: 0, y: 0, width: 1280, height: 32 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Layer,
                    },
                ),
            ]),
        });

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        {
            let world = app.inner().world();
            let damage = world.resource::<OutputDamageRegions>();
            let state = world.resource::<DamageState>();
            assert!(state.full_redraw);
            assert!(damage.regions[&virtual_id].contains(&nekoland_ecs::resources::DamageRect {
                x: 10,
                y: 15,
                width: 100,
                height: 80,
            }));
            assert!(damage.regions[&virtual_id].contains(&nekoland_ecs::resources::DamageRect {
                x: 0,
                y: 0,
                width: 1280,
                height: 32,
            }));
            assert!(damage.regions[&hdmi_id].contains(&nekoland_ecs::resources::DamageRect {
                x: 20,
                y: 30,
                width: 120,
                height: 90,
            }));
            assert!(damage.regions[&hdmi_id].contains(&nekoland_ecs::resources::DamageRect {
                x: 5,
                y: 7,
                width: 30,
                height: 20,
            }));
        }

        let Some(mut visibility) =
            app.inner_mut().world_mut().get_mut::<WindowViewportVisibility>(secondary_window)
        else {
            panic!("window viewport visibility should remain present");
        };
        visibility.output = Some(virtual_id);
        if let Some(mut snapshot) =
            app.inner_mut().world_mut().get_resource_mut::<SurfacePresentationSnapshot>()
        {
            if let Some(state) = snapshot.surfaces.get_mut(&2) {
                state.target_output = Some(virtual_id);
            }
            if let Some(state) = snapshot.surfaces.get_mut(&3) {
                state.target_output = Some(virtual_id);
            }
        }
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw, "output routing changes should trigger a redraw");
        assert!(damage.regions[&hdmi_id].contains(&nekoland_ecs::resources::DamageRect {
            x: 20,
            y: 30,
            width: 120,
            height: 90,
        }));
        assert!(damage.regions[&hdmi_id].contains(&nekoland_ecs::resources::DamageRect {
            x: 5,
            y: 7,
            width: 30,
            height: 20,
        }));
        assert_eq!(
            damage.regions[&virtual_id]
                .iter()
                .filter(|rect| rect.width == 120 && rect.height == 90)
                .count(),
            1,
        );
        assert_eq!(
            damage.regions[&virtual_id]
                .iter()
                .filter(|rect| rect.width == 30 && rect.height == 20)
                .count(),
            1,
        );

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(!state.full_redraw, "unchanged output routing should not trigger redraw");
        assert!(damage.regions.values().all(|rects| rects.is_empty()));
    }

    #[test]
    fn resizing_window_emits_border_damage_instead_of_full_window_redraw() {
        let mut app = NekolandApp::new("damage-tracker-resize-diff-test");
        install_damage_pipeline(&mut app);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: "one".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let virtual_id = output_id_by_name(app.inner_mut().world_mut(), "Virtual-1");

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 10 },
                geometry: SurfaceGeometry { x: 10, y: 20, width: 100, height: 80 },
                viewport_visibility: WindowViewportVisibility {
                    visible: true,
                    output: Some(virtual_id),
                },
                window: XdgWindow::default(),
                ..Default::default()
            })
            .id();
        app.inner_mut().world_mut().insert_resource(SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([(
                10,
                SurfacePresentationState {
                    visible: true,
                    target_output: Some(virtual_id),
                    geometry: SurfaceGeometry { x: 10, y: 20, width: 100, height: 80 },
                    input_enabled: true,
                    damage_enabled: true,
                    role: SurfacePresentationRole::Window,
                },
            )]),
        });

        app.inner_mut().world_mut().run_schedule(RenderSchedule);
        app.inner_mut().world_mut().resource_mut::<OutputDamageRegions>().regions.clear();
        app.inner_mut().world_mut().resource_mut::<DamageState>().full_redraw = false;

        let Some(mut geometry) = app.inner_mut().world_mut().get_mut::<SurfaceGeometry>(window)
        else {
            panic!("window geometry should remain present");
        };
        geometry.width = 120;
        if let Some(mut snapshot) =
            app.inner_mut().world_mut().get_resource_mut::<SurfacePresentationSnapshot>()
            && let Some(state) = snapshot.surfaces.get_mut(&10)
        {
            state.geometry.width = 120;
        }
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw);
        assert_eq!(
            damage.regions[&virtual_id],
            vec![nekoland_ecs::resources::DamageRect { x: 110, y: 20, width: 20, height: 80 }],
        );
    }

    #[test]
    fn content_commit_without_geometry_change_damages_current_rect() {
        let mut app = NekolandApp::new("damage-tracker-content-commit-test");
        install_damage_pipeline(&mut app);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: "one".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let virtual_id = output_id_by_name(app.inner_mut().world_mut(), "Virtual-1");

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 10 },
                geometry: SurfaceGeometry { x: 10, y: 20, width: 100, height: 80 },
                viewport_visibility: WindowViewportVisibility {
                    visible: true,
                    output: Some(virtual_id),
                },
                window: XdgWindow::default(),
                ..Default::default()
            })
            .id();
        app.inner_mut().world_mut().insert_resource(SurfacePresentationSnapshot {
            surfaces: std::collections::BTreeMap::from([(
                10,
                SurfacePresentationState {
                    visible: true,
                    target_output: Some(virtual_id),
                    geometry: SurfaceGeometry { x: 10, y: 20, width: 100, height: 80 },
                    input_enabled: true,
                    damage_enabled: true,
                    role: SurfacePresentationRole::Window,
                },
            )]),
        });

        app.inner_mut().world_mut().run_schedule(RenderSchedule);
        app.inner_mut().world_mut().resource_mut::<OutputDamageRegions>().regions.clear();
        app.inner_mut().world_mut().resource_mut::<DamageState>().full_redraw = false;

        let Some(mut content_version) =
            app.inner_mut().world_mut().get_mut::<SurfaceContentVersion>(window)
        else {
            panic!("surface content version should remain present");
        };
        content_version.bump();
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw);
        assert_eq!(
            damage.regions[&virtual_id],
            vec![nekoland_ecs::resources::DamageRect { x: 10, y: 20, width: 100, height: 80 }],
        );
    }

    #[test]
    fn solid_rect_injections_participate_in_damage_diff() {
        let mut app = NekolandApp::new("damage-tracker-solid-rect-test");
        install_damage_pipeline(&mut app);

        app.inner_mut().world_mut().spawn(OutputBundle {
            output: OutputDevice {
                name: "Virtual-1".to_owned(),
                kind: OutputKind::Virtual,
                make: "Virtual".to_owned(),
                model: "one".to_owned(),
            },
            properties: OutputProperties {
                width: 1280,
                height: 720,
                refresh_millihz: 60_000,
                scale: 1,
            },
            ..Default::default()
        });
        let virtual_id = output_id_by_name(app.inner_mut().world_mut(), "Virtual-1");
        app.inner_mut().world_mut().resource_mut::<ExternalSceneContributionState>().outputs =
            std::collections::BTreeMap::from([(
                virtual_id,
                vec![RenderSceneContribution {
                    key: RenderInstanceKey::new(
                        RenderSourceKey::new("test", "solid_rect"),
                        virtual_id,
                        0,
                    ),
                    payload: RenderSceneContributionPayload::SolidRect {
                        color: RenderColor { r: 1, g: 2, b: 3, a: 200 },
                    },
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 10, y: 20, width: 40, height: 30 },
                        opacity: 0.8,
                        clip_rect: None,
                        z_index: 1,
                        scene_role: RenderSceneRole::Overlay,
                    },
                }],
            )]);

        app.inner_mut().world_mut().run_schedule(RenderSchedule);
        {
            let world = app.inner().world();
            let damage = world.resource::<OutputDamageRegions>();
            let state = world.resource::<DamageState>();
            assert!(state.full_redraw);
            assert_eq!(
                damage.regions[&virtual_id],
                vec![nekoland_ecs::resources::DamageRect { x: 10, y: 20, width: 40, height: 30 }],
            );
        }

        app.inner_mut().world_mut().resource_mut::<OutputDamageRegions>().regions.clear();
        app.inner_mut().world_mut().resource_mut::<DamageState>().full_redraw = false;
        app.inner_mut().world_mut().resource_mut::<ExternalSceneContributionState>().outputs =
            std::collections::BTreeMap::from([(
                virtual_id,
                vec![RenderSceneContribution {
                    key: RenderInstanceKey::new(
                        RenderSourceKey::new("test", "solid_rect"),
                        virtual_id,
                        0,
                    ),
                    payload: RenderSceneContributionPayload::SolidRect {
                        color: RenderColor { r: 1, g: 2, b: 3, a: 200 },
                    },
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 20, y: 20, width: 40, height: 30 },
                        opacity: 0.8,
                        clip_rect: None,
                        z_index: 1,
                        scene_role: RenderSceneRole::Overlay,
                    },
                }],
            )]);
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw);
        assert_eq!(
            damage.regions[&virtual_id],
            vec![
                nekoland_ecs::resources::DamageRect { x: 10, y: 20, width: 10, height: 30 },
                nekoland_ecs::resources::DamageRect { x: 50, y: 20, width: 10, height: 30 },
            ],
        );
    }
}
