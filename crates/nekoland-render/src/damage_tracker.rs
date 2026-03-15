use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::{Entity, Local, Query, Res, ResMut, With};
use nekoland_ecs::components::{
    BufferState, DesiredOutputName, LayerOnOutput, LayerShellSurface, OutputDevice,
    SurfaceContentVersion, SurfaceGeometry, WindowMode, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{DamageRect, DamageState, OutputDamageRegions, PrimaryOutputState};
use nekoland_ecs::views::{PopupSnapshotRuntime, WindowSnapshotRuntime};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DamageTracker;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrackedSurfaceDamage {
    rect: DamageRect,
    content_version: u64,
}

type OutputDamageSnapshot = BTreeMap<String, BTreeMap<u64, TrackedSurfaceDamage>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DamageTrackerState {
    previous_snapshot: Option<OutputDamageSnapshot>,
}

/// Derives per-output scene damage from changes in the visible compositor scene graph.
///
/// The tracker keeps the previous output-local geometry plus a content-commit version for each
/// visible surface. Geometry-only changes emit a symmetric difference; content commits emit the
/// full current rect, and disappearing geometry emits the previous rect.
pub(crate) fn damage_tracking_system(
    layers: Query<
        (
            &WlSurfaceHandle,
            &SurfaceGeometry,
            &BufferState,
            &SurfaceContentVersion,
            Option<&LayerOnOutput>,
            Option<&DesiredOutputName>,
        ),
        With<LayerShellSurface>,
    >,
    windows: Query<(Entity, WindowSnapshotRuntime), With<XdgWindow>>,
    popups: Query<PopupSnapshotRuntime, With<XdgPopup>>,
    outputs: Query<(Entity, &OutputDevice)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    mut damage_state: ResMut<DamageState>,
    mut output_damage_regions: ResMut<OutputDamageRegions>,
    mut tracker_state: Local<DamageTrackerState>,
) {
    let live_output_names =
        outputs.iter().map(|(_, output)| output.name.clone()).collect::<BTreeSet<_>>();
    let output_names_by_entity = outputs
        .iter()
        .map(|(entity, output)| (entity, output.name.clone()))
        .collect::<HashMap<_, _>>();
    let primary_output_name = primary_output
        .and_then(|primary_output| primary_output.name.clone())
        .or_else(|| live_output_names.iter().next().cloned());
    let fallback_output_name =
        live_output_names.iter().next().cloned().unwrap_or_else(|| "Virtual-1".to_owned());
    let mut current_snapshot = if live_output_names.is_empty() {
        BTreeMap::from([(fallback_output_name.clone(), BTreeMap::new())])
    } else {
        live_output_names
            .iter()
            .cloned()
            .map(|output_name| (output_name, BTreeMap::new()))
            .collect::<OutputDamageSnapshot>()
    };

    let visible_windows = windows
        .iter()
        .filter(|(_, window)| {
            *window.mode != WindowMode::Hidden && window.viewport_visibility.visible
        })
        .map(|(entity, window)| {
            (
                entity,
                window.surface_id(),
                window.background.is_some(),
                window
                    .background
                    .map(|background| background.output.clone())
                    .or_else(|| window.viewport_visibility.output.clone()),
                TrackedSurfaceDamage {
                    rect: DamageRect {
                        x: window.geometry.x,
                        y: window.geometry.y,
                        width: window.geometry.width,
                        height: window.geometry.height,
                    },
                    content_version: window.content_version.value,
                },
            )
        })
        .collect::<Vec<_>>();
    let visible_windows = visible_windows
        .into_iter()
        .fold(BTreeMap::new(), |mut deduped, window| {
            let (entity, surface_id, is_background, output_name, rect) = window;
            let dedupe_key = is_background
                .then(|| output_name.clone())
                .flatten()
                .map(|output_name| format!("background:{output_name}"));
            if let Some(dedupe_key) = dedupe_key {
                match deduped.get(&dedupe_key) {
                    Some((_, current_surface_id, _, _)) if *current_surface_id >= surface_id => {}
                    _ => {
                        deduped.insert(dedupe_key, (entity, surface_id, output_name, rect));
                    }
                }
            } else {
                deduped.insert(
                    format!("window:{surface_id}"),
                    (entity, surface_id, output_name, rect),
                );
            }
            deduped
        })
        .into_values()
        .collect::<Vec<_>>();
    let active_window_entities =
        visible_windows.iter().map(|(entity, ..)| *entity).collect::<BTreeSet<_>>();
    let window_output_names = visible_windows
        .iter()
        .map(|(entity, _, output_name, _)| (*entity, output_name.clone()))
        .collect::<HashMap<_, _>>();

    for (_, surface_id, output_name, rect) in &visible_windows {
        record_surface_geometry(
            &mut current_snapshot,
            *surface_id,
            output_name.clone(),
            &live_output_names,
            &fallback_output_name,
            rect.clone(),
        );
    }

    for (surface, geometry, buffer, content_version, layer_output, desired_output_name) in &layers {
        if !buffer.attached {
            continue;
        }

        let output_name = layer_output
            .and_then(|layer_output| output_names_by_entity.get(&layer_output.0).cloned())
            .or_else(|| {
                desired_output_name.and_then(|desired_output_name| desired_output_name.0.clone())
            })
            .or_else(|| primary_output_name.clone());
        record_surface_geometry(
            &mut current_snapshot,
            surface.id,
            output_name,
            &live_output_names,
            &fallback_output_name,
            TrackedSurfaceDamage {
                rect: DamageRect {
                    x: geometry.x,
                    y: geometry.y,
                    width: geometry.width,
                    height: geometry.height,
                },
                content_version: content_version.value,
            },
        );
    }

    for popup in &popups {
        if !popup.buffer.attached || !popup_parent_visible(popup.child_of, &active_window_entities)
        {
            continue;
        }

        let output_name = window_output_names.get(&popup.child_of.parent()).cloned().flatten();
        record_surface_geometry(
            &mut current_snapshot,
            popup.surface_id(),
            output_name,
            &live_output_names,
            &fallback_output_name,
            TrackedSurfaceDamage {
                rect: DamageRect {
                    x: popup.geometry.x,
                    y: popup.geometry.y,
                    width: popup.geometry.width,
                    height: popup.geometry.height,
                },
                content_version: popup.content_version.value,
            },
        );
    }

    let damage_regions = tracker_state
        .previous_snapshot
        .as_ref()
        .map(|previous_snapshot| diff_damage_snapshots(previous_snapshot, &current_snapshot))
        .unwrap_or_else(|| {
            current_snapshot
                .iter()
                .map(|(output_name, surfaces)| {
                    (
                        output_name.clone(),
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

/// Popups only contribute damage while their parent toplevel is still visible.
fn popup_parent_visible(child_of: &ChildOf, active_window_entities: &BTreeSet<Entity>) -> bool {
    active_window_entities.contains(&child_of.parent())
}

fn record_surface_geometry(
    snapshot: &mut OutputDamageSnapshot,
    surface_id: u64,
    output_name: Option<String>,
    live_output_names: &BTreeSet<String>,
    fallback_output_name: &str,
    surface: TrackedSurfaceDamage,
) {
    if live_output_names.is_empty() {
        snapshot.entry(fallback_output_name.to_owned()).or_default().insert(surface_id, surface);
        return;
    }

    let Some(output_name) =
        output_name.filter(|output_name| live_output_names.contains(output_name))
    else {
        return;
    };
    snapshot.entry(output_name).or_default().insert(surface_id, surface);
}

fn diff_damage_snapshots(
    previous_snapshot: &OutputDamageSnapshot,
    current_snapshot: &OutputDamageSnapshot,
) -> BTreeMap<String, Vec<DamageRect>> {
    current_snapshot
        .iter()
        .map(|(output_name, current_surfaces)| {
            let previous_surfaces = previous_snapshot.get(output_name);
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
                        if previous.content_version != current.content_version {
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

            (output_name.clone(), normalize_damage_rects(damage))
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
    use bevy_ecs::hierarchy::ChildOf;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        BufferState, DesiredOutputName, LayerShellSurface, OutputDevice, OutputKind,
        OutputProperties, SurfaceContentVersion, SurfaceGeometry, WindowViewportVisibility,
        WlSurfaceHandle, XdgPopup, XdgWindow,
    };
    use nekoland_ecs::resources::{DamageState, OutputDamageRegions};

    use super::damage_tracking_system;

    #[test]
    fn damage_regions_are_scoped_per_output_and_recompute_on_output_change() {
        let mut app = NekolandApp::new("damage-tracker-output-routing-test");
        app.inner_mut()
            .init_resource::<DamageState>()
            .init_resource::<OutputDamageRegions>()
            .add_systems(RenderSchedule, damage_tracking_system);

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

        let secondary_window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 2 },
                geometry: SurfaceGeometry { x: 20, y: 30, width: 120, height: 90 },
                viewport_visibility: WindowViewportVisibility {
                    visible: true,
                    output: Some("HDMI-A-1".to_owned()),
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
                output: Some("Virtual-1".to_owned()),
            },
            window: XdgWindow::default(),
            ..Default::default()
        });
        app.inner_mut().world_mut().spawn((
            WlSurfaceHandle { id: 3 },
            XdgPopup::default(),
            BufferState { attached: true, scale: 1 },
            SurfaceGeometry { x: 5, y: 7, width: 30, height: 20 },
            ChildOf(secondary_window),
        ));
        app.inner_mut().world_mut().spawn((
            LayerShellSurface::default(),
            WlSurfaceHandle { id: 4 },
            SurfaceGeometry { x: 0, y: 0, width: 1280, height: 32 },
            BufferState { attached: true, scale: 1 },
            DesiredOutputName(Some("Virtual-1".to_owned())),
        ));

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        {
            let world = app.inner().world();
            let damage = world.resource::<OutputDamageRegions>();
            let state = world.resource::<DamageState>();
            assert!(state.full_redraw);
            assert!(damage.regions["Virtual-1"].contains(&nekoland_ecs::resources::DamageRect {
                x: 10,
                y: 15,
                width: 100,
                height: 80,
            }));
            assert!(damage.regions["Virtual-1"].contains(&nekoland_ecs::resources::DamageRect {
                x: 0,
                y: 0,
                width: 1280,
                height: 32,
            }));
            assert!(damage.regions["HDMI-A-1"].contains(&nekoland_ecs::resources::DamageRect {
                x: 20,
                y: 30,
                width: 120,
                height: 90,
            }));
            assert!(damage.regions["HDMI-A-1"].contains(&nekoland_ecs::resources::DamageRect {
                x: 5,
                y: 7,
                width: 30,
                height: 20,
            }));
        }

        app.inner_mut()
            .world_mut()
            .get_mut::<WindowViewportVisibility>(secondary_window)
            .expect("window viewport visibility should remain present")
            .output = Some("Virtual-1".to_owned());
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw, "output routing changes should trigger a redraw");
        assert!(damage.regions["HDMI-A-1"].contains(&nekoland_ecs::resources::DamageRect {
            x: 20,
            y: 30,
            width: 120,
            height: 90,
        }));
        assert!(damage.regions["HDMI-A-1"].contains(&nekoland_ecs::resources::DamageRect {
            x: 5,
            y: 7,
            width: 30,
            height: 20,
        }));
        assert_eq!(
            damage.regions["Virtual-1"]
                .iter()
                .filter(|rect| rect.width == 120 && rect.height == 90)
                .count(),
            1,
        );
        assert_eq!(
            damage.regions["Virtual-1"]
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
        app.inner_mut()
            .init_resource::<DamageState>()
            .init_resource::<OutputDamageRegions>()
            .add_systems(RenderSchedule, damage_tracking_system);

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

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 10 },
                geometry: SurfaceGeometry { x: 10, y: 20, width: 100, height: 80 },
                viewport_visibility: WindowViewportVisibility {
                    visible: true,
                    output: Some("Virtual-1".to_owned()),
                },
                window: XdgWindow::default(),
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(RenderSchedule);
        app.inner_mut().world_mut().resource_mut::<OutputDamageRegions>().regions.clear();
        app.inner_mut().world_mut().resource_mut::<DamageState>().full_redraw = false;

        app.inner_mut()
            .world_mut()
            .get_mut::<SurfaceGeometry>(window)
            .expect("window geometry should remain present")
            .width = 120;
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw);
        assert_eq!(
            damage.regions["Virtual-1"],
            vec![nekoland_ecs::resources::DamageRect { x: 110, y: 20, width: 20, height: 80 }],
        );
    }

    #[test]
    fn content_commit_without_geometry_change_damages_current_rect() {
        let mut app = NekolandApp::new("damage-tracker-content-commit-test");
        app.inner_mut()
            .init_resource::<DamageState>()
            .init_resource::<OutputDamageRegions>()
            .add_systems(RenderSchedule, damage_tracking_system);

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

        let window = app
            .inner_mut()
            .world_mut()
            .spawn(WindowBundle {
                surface: WlSurfaceHandle { id: 10 },
                geometry: SurfaceGeometry { x: 10, y: 20, width: 100, height: 80 },
                viewport_visibility: WindowViewportVisibility {
                    visible: true,
                    output: Some("Virtual-1".to_owned()),
                },
                window: XdgWindow::default(),
                ..Default::default()
            })
            .id();

        app.inner_mut().world_mut().run_schedule(RenderSchedule);
        app.inner_mut().world_mut().resource_mut::<OutputDamageRegions>().regions.clear();
        app.inner_mut().world_mut().resource_mut::<DamageState>().full_redraw = false;

        app.inner_mut()
            .world_mut()
            .get_mut::<SurfaceContentVersion>(window)
            .expect("surface content version should remain present")
            .bump();
        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let world = app.inner().world();
        let damage = world.resource::<OutputDamageRegions>();
        let state = world.resource::<DamageState>();
        assert!(state.full_redraw);
        assert_eq!(
            damage.regions["Virtual-1"],
            vec![nekoland_ecs::resources::DamageRect { x: 10, y: 20, width: 100, height: 80 }],
        );
    }
}
