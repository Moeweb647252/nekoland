use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::entity::Entity;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, NonSend, Query, Res, ResMut, Resource, With};
use bevy_ecs::system::SystemParam;
use nekoland_core::error::NekolandError;
use nekoland_ecs::bundles::OutputBundle;
use nekoland_ecs::components::{
    OutputDevice, OutputId, OutputPlacement, OutputProperties, OutputViewport,
    OutputWorkArea, WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::kinds::{BackendEvent, FrameQueue};
use nekoland_ecs::resources::{
    BackendOutputRegistry, CompositorConfig, ConfiguredOutput, EntityIndex, FocusedOutputState,
    OutputOverlayState, OutputServerAction, OutputServerRequest, PendingOutputControl,
    PendingOutputControls, PendingOutputOverlayControls, PendingOutputServerRequests,
    PrimaryOutputState,
};
use nekoland_ecs::selectors::OutputSelector;
use nekoland_ecs::views::OutputRuntime;
use serde::{Deserialize, Serialize};

use crate::components::OutputBackend;
use crate::manager::BackendManager;
use crate::traits::{BackendId, BackendOutputId, OutputSnapshot};

/// Remembers output-local viewport origins across output disable/enable and reconnect cycles.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct RememberedOutputViewportState {
    pub by_id: BTreeMap<OutputId, OutputViewport>,
    pub ids_by_name: BTreeMap<String, OutputId>,
}

impl RememberedOutputViewportState {
    pub fn viewport_for_output_id(&self, output_id: OutputId) -> Option<&OutputViewport> {
        self.by_id.get(&output_id)
    }

    pub fn viewport_for_output_name(&self, output_name: &str) -> Option<&OutputViewport> {
        self.ids_by_name.get(output_name).and_then(|output_id| self.by_id.get(output_id))
    }

    pub fn remember(&mut self, output_id: OutputId, output_name: String, viewport: OutputViewport) {
        self.ids_by_name.insert(output_name, output_id);
        self.by_id.insert(output_id, viewport);
    }

    pub fn forget_name(&mut self, output_name: &str) {
        self.ids_by_name.remove(output_name);
    }
}

/// Output metadata that a backend runtime wants the ECS world to materialize.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputBlueprint {
    /// Backend-local opaque identity for the output within one runtime instance.
    pub local_id: String,
    /// Device identity inserted into ECS for the materialized output.
    pub device: OutputDevice,
    /// Initial output properties inserted alongside the device identity.
    pub properties: OutputProperties,
}

/// One backend-originated output lifecycle event carrying explicit backend ownership metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputEventRecord {
    /// Backend runtime that originated the event.
    pub backend_id: BackendId,
    /// Human-readable output name surfaced through device metadata and public events.
    pub output_name: String,
    /// Backend-local opaque identity used to match existing ECS entities.
    pub local_id: String,
    /// Connect or disconnect transition requested by the backend.
    pub change: BackendOutputChange,
}

/// Output connect/disconnect lifecycle, expressed in backend-normalized form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BackendOutputChange {
    Connected(BackendOutputBlueprint),
    Disconnected,
}

impl BackendEvent for BackendOutputEventRecord {}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendOutputUpdateQueueTag;

/// Per-frame queue of backend-originated output lifecycle events.
pub type PendingBackendOutputEvents =
    nekoland_ecs::kinds::BackendEventQueue<BackendOutputEventRecord>;

/// One backend-originated output property refresh to be applied to an already materialized ECS
/// output entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputPropertyUpdate {
    /// Backend runtime that owns the output being refreshed.
    pub backend_id: BackendId,
    /// Human-readable output name associated with the refreshed output.
    pub output_name: String,
    /// Backend-local opaque identity used to find the ECS entity to update.
    pub local_id: String,
    /// Replacement output properties produced by the backend extract phase.
    pub properties: OutputProperties,
}

/// Per-frame queue of output property refreshes produced during backend extract.
pub type PendingBackendOutputUpdates =
    FrameQueue<BackendOutputPropertyUpdate, BackendOutputUpdateQueueTag>;

type OutputViewportQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static OutputId,
        &'static OutputDevice,
        &'static OutputProperties,
        &'static mut OutputViewport,
    ),
>;
type OutputWindowSceneQuery<'w, 's> =
    Query<'w, 's, (&'static WlSurfaceHandle, &'static WindowSceneGeometry), With<XdgWindow>>;
type ManagedOutputQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static OutputId,
        &'static OutputDevice,
        &'static OutputBackend,
        &'static mut OutputProperties,
    ),
>;

#[derive(SystemParam)]
pub(crate) struct OutputControlRequestCtx<'w, 's> {
    pending_output_controls: ResMut<'w, PendingOutputControls>,
    pending_output_overlay_controls: ResMut<'w, PendingOutputOverlayControls>,
    pending_output_requests: ResMut<'w, PendingOutputServerRequests>,
    primary_output: Res<'w, PrimaryOutputState>,
    focused_output: Res<'w, FocusedOutputState>,
    entity_index: Res<'w, EntityIndex>,
    remembered_viewports: ResMut<'w, RememberedOutputViewportState>,
    outputs: OutputViewportQuery<'w, 's>,
    windows: OutputWindowSceneQuery<'w, 's>,
}

#[derive(SystemParam)]
pub(crate) struct OutputServerRequestCtx<'w, 's> {
    commands: Commands<'w, 's>,
    manager: NonSend<'w, BackendManager>,
    output_registry: ResMut<'w, BackendOutputRegistry>,
    remembered_viewports: Res<'w, RememberedOutputViewportState>,
    pending_output_requests: ResMut<'w, PendingOutputServerRequests>,
    outputs: ManagedOutputQuery<'w, 's>,
    output_connected: MessageWriter<'w, OutputConnected>,
    output_disconnected: MessageWriter<'w, OutputDisconnected>,
}

/// Translates the latest config snapshot into idempotent enable/configure/disable requests for
/// the backend-facing output controller.
pub fn sync_configured_outputs_system(
    config: Option<Res<CompositorConfig>>,
    outputs: Query<OutputRuntime>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
    mut last_applied_outputs: bevy_ecs::prelude::Local<Option<Vec<ConfiguredOutput>>>,
) -> bevy_ecs::error::Result {
    let Some(config) = config else {
        return Ok(());
    };

    if last_applied_outputs.as_ref() == Some(&config.outputs) {
        return Ok(());
    }

    if config.outputs.is_empty() {
        *last_applied_outputs = Some(Vec::new());
        return Ok(());
    }

    let existing_outputs = outputs
        .iter()
        .map(|output| (output.name().to_owned(), output.properties.clone()))
        .collect::<BTreeMap<_, _>>();

    let invalid_outputs = enqueue_configured_output_requests(
        &config,
        &existing_outputs,
        &mut pending_output_requests,
    );
    let configured_names =
        config.outputs.iter().map(|output| output.name.as_str()).collect::<BTreeSet<_>>();
    for output_name in existing_outputs.keys() {
        if !configured_names.contains(output_name.as_str()) {
            pending_output_requests.push(OutputServerRequest {
                action: OutputServerAction::Disable { output: output_name.clone() },
            });
        }
    }

    *last_applied_outputs = Some(config.outputs.clone());

    if invalid_outputs.is_empty() {
        Ok(())
    } else {
        Err(NekolandError::Config(format!(
            "ignored invalid configured output modes: {}",
            invalid_outputs.join(", ")
        ))
        .into())
    }
}

/// Folds high-level output controls from IPC/keybindings into the backend-facing request queue
/// that config sync and runtime application already use.
pub(crate) fn apply_output_control_requests_system(ctx: OutputControlRequestCtx<'_, '_>) {
    let OutputControlRequestCtx {
        mut pending_output_controls,
        mut pending_output_overlay_controls,
        mut pending_output_requests,
        primary_output,
        focused_output,
        entity_index,
        mut remembered_viewports,
        mut outputs,
        windows,
    } = ctx;
    let mut deferred = Vec::new();

    for control in pending_output_controls.take() {
        let output = match &control.selector {
            OutputSelector::Id(output_id) => {
                let Some((_, device, _, _)) =
                    outputs.iter_mut().find(|(candidate_id, _, _, _)| *candidate_id == output_id)
                else {
                    deferred.push(control);
                    continue;
                };
                device.name.clone()
            }
            OutputSelector::Name(output) => output.as_str().to_owned(),
            OutputSelector::Primary => {
                let Some(output) = primary_output.id.and_then(|output_id| {
                    outputs.iter_mut().find_map(|(candidate_id, device, _, _)| {
                        (*candidate_id == output_id).then(|| device.name.clone())
                    })
                }) else {
                    deferred.push(control);
                    continue;
                };
                output
            }
            OutputSelector::Focused => {
                let Some(output) = focused_output
                    .id
                    .and_then(|output_id| {
                        outputs.iter_mut().find_map(|(candidate_id, device, _, _)| {
                            (*candidate_id == output_id).then(|| device.name.clone())
                        })
                    })
                    .or_else(|| {
                        primary_output.id.and_then(|output_id| {
                            outputs.iter_mut().find_map(|(candidate_id, device, _, _)| {
                                (*candidate_id == output_id).then(|| device.name.clone())
                            })
                        })
                    })
                else {
                    deferred.push(control);
                    continue;
                };
                output
            }
        };

        if let Some(enabled) = control.enabled {
            pending_output_requests.push(OutputServerRequest {
                action: if enabled {
                    OutputServerAction::Enable { output: output.clone() }
                } else {
                    OutputServerAction::Disable { output: output.clone() }
                },
            });
        }

        if let Some(configuration) = control.configuration {
            pending_output_requests.push(OutputServerRequest {
                action: OutputServerAction::Configure {
                    output: output.clone(),
                    mode: configuration.mode,
                    scale: configuration.scale,
                },
            });
        }

        let mut deferred_control = PendingOutputControl {
            selector: control.selector.clone(),
            enabled: None,
            configuration: None,
            viewport_origin: control.viewport_origin,
            viewport_pan: control.viewport_pan,
            center_viewport_on: control.center_viewport_on,
            clear_overlays: control.clear_overlays,
            overlay_updates: control.overlay_updates,
        };

        let Some((output_id, _, output_properties, mut viewport)) =
            outputs.iter_mut().find(|(_, device, _, _)| device.name == output)
        else {
            if deferred_control.viewport_origin.is_some()
                || deferred_control.viewport_pan.is_some()
                || deferred_control.center_viewport_on.is_some()
                || deferred_control.clear_overlays
                || !deferred_control.overlay_updates.is_empty()
            {
                deferred.push(deferred_control);
            }
            continue;
        };

        if let Some(origin) = deferred_control.viewport_origin.take() {
            viewport.move_to(origin.x, origin.y);
        }
        if let Some(pan) = deferred_control.viewport_pan.take() {
            viewport.pan_by(pan.delta_x, pan.delta_y);
        }
        if let Some(surface_id) = deferred_control.center_viewport_on.take() {
            let target_window = entity_index
                .entity_for_surface(surface_id.0)
                .and_then(|entity| windows.get(entity).ok())
                .or_else(|| windows.iter().find(|(surface, _)| surface.id == surface_id.0));
            if let Some((_, scene_geometry)) = target_window {
                center_viewport_on_scene_geometry(&mut viewport, scene_geometry, output_properties);
            } else {
                deferred_control.center_viewport_on = Some(surface_id);
            }
        }
        if deferred_control.clear_overlays {
            pending_output_overlay_controls.output(*output_id).clear_overlays();
            deferred_control.clear_overlays = false;
        }
        for overlay_update in std::mem::take(&mut deferred_control.overlay_updates) {
            let overlay_control = &mut pending_output_overlay_controls.output(*output_id);
            match overlay_update {
                nekoland_ecs::resources::OutputOverlayUpdate::Set(spec) => {
                    overlay_control.set_overlay(spec);
                }
                nekoland_ecs::resources::OutputOverlayUpdate::Remove(overlay_id) => {
                    overlay_control.remove_overlay(overlay_id);
                }
            }
        }
        remembered_viewports.remember(*output_id, output.clone(), viewport.clone());

        if deferred_control.viewport_origin.is_some()
            || deferred_control.viewport_pan.is_some()
            || deferred_control.center_viewport_on.is_some()
            || deferred_control.clear_overlays
            || !deferred_control.overlay_updates.is_empty()
        {
            deferred.push(deferred_control);
        }
    }

    pending_output_controls.replace(deferred);
}

/// Applies output-local overlay control updates after selectors have been resolved into `OutputId`.
pub(crate) fn apply_output_overlay_controls_system(
    mut pending_output_overlay_controls: ResMut<'_, PendingOutputOverlayControls>,
    mut output_overlays: ResMut<'_, OutputOverlayState>,
) {
    for control in pending_output_overlay_controls.take() {
        if control.clear_overlays {
            output_overlays.clear_output(control.output_id);
        }

        for update in control.overlay_updates {
            match update {
                nekoland_ecs::resources::OutputOverlayUpdate::Set(spec) => {
                    output_overlays.upsert(control.output_id, spec);
                }
                nekoland_ecs::resources::OutputOverlayUpdate::Remove(overlay_id) => {
                    output_overlays.remove(control.output_id, &overlay_id);
                }
            }
        }
    }
}

pub fn enqueue_configured_output_requests(
    config: &CompositorConfig,
    existing_outputs: &BTreeMap<String, OutputProperties>,
    pending_output_requests: &mut PendingOutputServerRequests,
) -> Vec<String> {
    let mut invalid_outputs = Vec::new();

    for configured_output in &config.outputs {
        if configured_output.enabled {
            if parse_output_mode(&configured_output.mode).is_none() {
                invalid_outputs
                    .push(format!("{}={}", configured_output.name, configured_output.mode));
                continue;
            }

            if !existing_outputs.contains_key(&configured_output.name) {
                pending_output_requests.push(OutputServerRequest {
                    action: OutputServerAction::Enable { output: configured_output.name.clone() },
                });
            }

            if existing_outputs
                .get(&configured_output.name)
                .is_none_or(|properties| !output_matches_config(properties, configured_output))
            {
                pending_output_requests.push(OutputServerRequest {
                    action: OutputServerAction::Configure {
                        output: configured_output.name.clone(),
                        mode: configured_output.mode.clone(),
                        scale: Some(configured_output.scale.max(1)),
                    },
                });
            }
        } else if existing_outputs.contains_key(&configured_output.name) {
            pending_output_requests.push(OutputServerRequest {
                action: OutputServerAction::Disable { output: configured_output.name.clone() },
            });
        }
    }

    invalid_outputs
}

/// Materializes backend output announcements into ECS entities and removes stale ones when a
/// backend reports a disconnect.
pub fn synchronize_backend_outputs_system(
    mut commands: Commands,
    mut output_registry: ResMut<BackendOutputRegistry>,
    mut remembered_viewports: ResMut<RememberedOutputViewportState>,
    mut pending_output_events: ResMut<PendingBackendOutputEvents>,
    existing_outputs: Query<(Entity, &OutputId, &OutputDevice, &OutputBackend)>,
    mut output_connected: MessageWriter<OutputConnected>,
    mut output_disconnected: MessageWriter<OutputDisconnected>,
) {
    for (_, output_id, output, _) in &existing_outputs {
        output_registry.remember_connected(*output_id, output.name.clone());
        output_registry.remember_enabled(*output_id, output.name.clone());
    }

    for record in pending_output_events.drain() {
        match record.change {
            BackendOutputChange::Connected(blueprint) => {
                if let Some((entity, output_id, _, _)) =
                    existing_outputs.iter().find(|(_, _, _, owner)| {
                        owner.output_id
                            == (BackendOutputId {
                                backend_id: record.backend_id,
                                local_id: record.local_id.clone(),
                            })
                    })
                {
                    commands
                        .entity(entity)
                        .insert((blueprint.device.clone(), blueprint.properties));
                    output_registry.remember_connected(*output_id, record.output_name.clone());
                    output_registry.remember_enabled(*output_id, record.output_name.clone());
                    continue;
                }

                let output_id = OutputId::fresh();
                commands.spawn((
                    output_id,
                    OutputBundle {
                        output: blueprint.device,
                        properties: blueprint.properties,
                        viewport: remembered_viewports
                            .viewport_for_output_name(&record.output_name)
                            .cloned()
                            .unwrap_or_default(),
                        ..Default::default()
                    },
                    OutputBackend {
                        backend_id: record.backend_id,
                        output_id: BackendOutputId {
                            backend_id: record.backend_id,
                            local_id: blueprint.local_id.clone(),
                        },
                    },
                ));
                output_registry.remember_connected(output_id, record.output_name.clone());
                output_registry.remember_enabled(output_id, record.output_name.clone());
                output_connected.write(OutputConnected { name: record.output_name.clone() });
            }
            BackendOutputChange::Disconnected => {
                for (entity, _, output, owner) in &existing_outputs {
                    if owner.output_id
                        == (BackendOutputId {
                            backend_id: record.backend_id,
                            local_id: record.local_id.clone(),
                        })
                    {
                        commands.entity(entity).despawn();
                        output_registry.forget_connected_name(&output.name);
                        remembered_viewports.forget_name(&output.name);
                    }
                }
                output_disconnected.write(OutputDisconnected { name: record.output_name.clone() });
                output_registry.forget_connected_name(&record.output_name);
                remembered_viewports.forget_name(&record.output_name);
            }
        }
    }
}

/// Applies backend-originated property refreshes to already-materialized ECS output entities.
pub fn apply_backend_output_updates_system(
    mut outputs: Query<(&OutputDevice, &OutputBackend, &mut OutputProperties)>,
    mut pending_updates: ResMut<PendingBackendOutputUpdates>,
) {
    for update in pending_updates.drain() {
        for (_, owner, mut properties) in &mut outputs {
            if owner.output_id
                == (BackendOutputId {
                    backend_id: update.backend_id,
                    local_id: update.local_id.clone(),
                })
            {
                *properties = update.properties.clone();
                break;
            }
        }
    }
}

/// Derives deterministic output placement plus a full-size base work area before shell layout
/// systems carve out exclusive zones.
pub fn sync_output_layout_state_system(
    mut outputs: Query<(
        &OutputDevice,
        &OutputProperties,
        &mut OutputPlacement,
        &mut OutputWorkArea,
    )>,
) {
    let mut snapshots = outputs
        .iter()
        .map(|(device, properties, _, _)| {
            (device.name.clone(), properties.width.max(1), properties.height.max(1))
        })
        .collect::<Vec<_>>();
    snapshots.sort_by(|left, right| left.0.cmp(&right.0));

    let mut next_x = 0_i32;
    let placements = snapshots
        .into_iter()
        .map(|(name, width, height)| {
            let placement = OutputPlacement { x: next_x, y: 0 };
            let work_area = OutputWorkArea { x: 0, y: 0, width, height };
            next_x = next_x.saturating_add(width.clamp(1, i32::MAX as u32) as i32);
            (name, (placement, work_area))
        })
        .collect::<BTreeMap<_, _>>();

    for (device, _, mut placement, mut work_area) in &mut outputs {
        if let Some((next_placement, next_work_area)) = placements.get(&device.name) {
            *placement = next_placement.clone();
            *work_area = next_work_area.clone();
        }
    }
}

/// Applies public output-management requests against ECS output state and explicit backend
/// ownership metadata.
/// Materialize public output-management requests against currently installed
/// backends, deferring requests that cannot yet be satisfied this frame.
pub(crate) fn apply_output_server_requests_system(ctx: OutputServerRequestCtx<'_, '_>) {
    let OutputServerRequestCtx {
        mut commands,
        manager,
        mut output_registry,
        remembered_viewports,
        mut pending_output_requests,
        mut outputs,
        mut output_connected,
        mut output_disconnected,
    } = ctx;
    let mut deferred = Vec::new();

    for request in pending_output_requests.drain() {
        match request.action {
            OutputServerAction::Enable { output } => {
                if outputs.iter().any(|(_, _, existing, _, _)| existing.name == output)
                    || output_registry.has_enabled_name(&output)
                {
                    continue;
                }

                let Some(seed) = manager.seed_output(&output) else {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Enable { output },
                    });
                    continue;
                };
                let crate::manager::SeededBackendOutput { backend_id, blueprint } = seed;
                let BackendOutputBlueprint { local_id, device, properties } = blueprint;

                let output_id = OutputId::fresh();
                commands.spawn((
                    output_id,
                    OutputBundle {
                        output: device,
                        properties,
                        viewport: remembered_viewports
                            .viewport_for_output_name(&output)
                            .cloned()
                            .unwrap_or_default(),
                        ..Default::default()
                    },
                    OutputBackend {
                        backend_id,
                        output_id: BackendOutputId { backend_id, local_id },
                    },
                ));
                output_registry.remember_connected(output_id, output.clone());
                output_registry.remember_enabled(output_id, output.clone());
                output_connected.write(OutputConnected { name: output });
            }
            OutputServerAction::Disable { output } => {
                let Some((entity, _, _, _, _)) =
                    outputs.iter_mut().find(|(_, _, existing, _, _)| existing.name == output)
                else {
                    continue;
                };

                commands.entity(entity).despawn();
                output_registry.forget_enabled_name(&output);
                output_disconnected.write(OutputDisconnected { name: output });
            }
            OutputServerAction::Configure { output, mode, scale } => {
                let Some(configured_mode) = parse_output_mode(&mode) else {
                    tracing::warn!(output, mode, "ignoring invalid output mode request");
                    continue;
                };

                let Some((_, _, _, _, mut properties)) =
                    outputs.iter_mut().find(|(_, _, existing, _, _)| existing.name == output)
                else {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Configure { output, mode, scale },
                    });
                    continue;
                };

                properties.width = configured_mode.width;
                properties.height = configured_mode.height;
                properties.refresh_millihz = configured_mode.refresh_millihz;
                if let Some(scale) = scale {
                    properties.scale = scale.max(1);
                }
            }
        }
    }

    pending_output_requests.replace(deferred);
}

pub fn remember_output_viewports_system(
    outputs: Query<(&OutputId, &OutputDevice, &OutputViewport)>,
    mut remembered_viewports: ResMut<RememberedOutputViewportState>,
) {
    for (output_id, output, viewport) in &outputs {
        remembered_viewports.remember(*output_id, output.name.clone(), viewport.clone());
    }
}

/// Refresh the public primary-output snapshot from current outputs and config preference order.
pub fn sync_primary_output_state_system(
    config: Option<Res<CompositorConfig>>,
    outputs: Query<OutputRuntime>,
    mut primary_output: ResMut<PrimaryOutputState>,
) {
    let next_primary = select_primary_output_target(config.as_deref(), &outputs);
    if primary_output.id != next_primary.as_ref().map(|(id, _)| *id) {
        tracing::trace!(
            previous = ?primary_output.id,
            next = ?next_primary.as_ref().map(|(id, _)| *id),
            "updated primary output"
        );
        primary_output.id = next_primary.as_ref().map(|(id, _)| *id);
    }
}

pub fn select_primary_output_target(
    config: Option<&CompositorConfig>,
    outputs: &Query<OutputRuntime>,
) -> Option<(OutputId, String)> {
    let next_name = select_primary_output_name(config, outputs)?;
    outputs
        .iter()
        .find(|output| output.name() == next_name)
        .map(|output| (output.id(), output.name().to_owned()))
}

/// Choose the primary output from live ECS outputs.
pub fn select_primary_output_name(
    config: Option<&CompositorConfig>,
    outputs: &Query<OutputRuntime>,
) -> Option<String> {
    let output_snapshots = outputs
        .iter()
        .map(|output| {
            (
                output.name().to_owned(),
                u64::from(output.properties.width.max(1)),
                u64::from(output.properties.height.max(1)),
            )
        })
        .collect::<Vec<_>>();

    select_primary_output_name_from_snapshots(config, &output_snapshots)
}

/// Choose the primary output from plain `(name, width, height)` snapshots.
///
/// Preference order is:
/// 1. First enabled configured output that currently exists
/// 2. Largest live output by pixel area, then by name for stability
pub fn select_primary_output_name_from_snapshots(
    config: Option<&CompositorConfig>,
    output_snapshots: &[(String, u64, u64)],
) -> Option<String> {
    if let Some(config) = config {
        for configured_output in
            config.outputs.iter().filter(|configured_output| configured_output.enabled)
        {
            if output_snapshots
                .iter()
                .any(|(output_name, _, _)| output_name == &configured_output.name)
            {
                return Some(configured_output.name.clone());
            }
        }
    }

    output_snapshots
        .iter()
        .cloned()
        .max_by(|(left_name, left_width, left_height), (right_name, right_width, right_height)| {
            let left_area = left_width.saturating_mul(*left_height);
            let right_area = right_width.saturating_mul(*right_height);
            left_area.cmp(&right_area).then_with(|| right_name.cmp(left_name))
        })
        .map(|(output_name, _, _)| output_name)
}

pub fn collect_output_snapshots(
    outputs: &Query<(Entity, OutputRuntime, Option<&OutputBackend>)>,
) -> Vec<OutputSnapshot> {
    outputs
        .iter()
        .map(|(entity, output, owner)| OutputSnapshot {
            entity,
            output_id: output.id(),
            backend_id: owner.map(|owner| owner.backend_id),
            backend_output_id: owner.map(|owner| owner.output_id.clone()),
            device: output.device.clone(),
            properties: output.properties.clone(),
        })
        .collect()
}

fn center_viewport_on_scene_geometry(
    viewport: &mut OutputViewport,
    scene_geometry: &WindowSceneGeometry,
    output: &OutputProperties,
) {
    let half_width = (output.width / 2) as isize;
    let half_height = (output.height / 2) as isize;
    let target_x = scene_geometry.x.saturating_add((scene_geometry.width / 2) as isize);
    let target_y = scene_geometry.y.saturating_add((scene_geometry.height / 2) as isize);
    viewport.origin_x = target_x.saturating_sub(half_width);
    viewport.origin_y = target_y.saturating_sub(half_height);
}

pub fn output_matches_config(
    properties: &OutputProperties,
    configured_output: &ConfiguredOutput,
) -> bool {
    parse_output_mode(&configured_output.mode).is_some_and(|mode| {
        properties.width == mode.width
            && properties.height == mode.height
            && properties.refresh_millihz == mode.refresh_millihz
            && properties.scale == configured_output.scale.max(1)
    })
}

#[derive(Clone, Copy)]
pub struct ParsedOutputMode {
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
}

pub fn parse_output_mode(mode: &str) -> Option<ParsedOutputMode> {
    let (dimensions, refresh_hz) = match mode.split_once('@') {
        Some((dimensions, refresh_hz)) => (dimensions, refresh_hz.parse::<u32>().ok()?),
        None => (mode, 60),
    };
    let (width, height) = dimensions.split_once('x')?;

    Some(ParsedOutputMode {
        width: width.parse::<u32>().ok()?.max(1),
        height: height.parse::<u32>().ok()?.max(1),
        refresh_millihz: refresh_hz.saturating_mul(1000),
    })
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::Messages;
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::ExtractSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputProperties, OutputViewport, WindowLayout,
        WindowMode, WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
    use nekoland_ecs::resources::{
        EntityIndex, FocusedOutputState, OutputOverlayId, OutputOverlayState,
        PendingOutputControls, PendingOutputOverlayControls, PendingOutputServerRequests,
        PrimaryOutputState, RenderColor, RenderRect,
    };
    use nekoland_ecs::selectors::{OutputName, OutputSelector, SurfaceId};

    use crate::components::OutputBackend;
    use crate::traits::BackendId;

    use super::{
        BackendOutputBlueprint, BackendOutputChange, BackendOutputEventRecord,
        BackendOutputRegistry, PendingBackendOutputEvents, RememberedOutputViewportState,
        apply_output_control_requests_system, apply_output_overlay_controls_system,
        remember_output_viewports_system, synchronize_backend_outputs_system,
    };

    #[test]
    fn viewport_controls_update_output_viewport_state() {
        let mut app = NekolandApp::new("output-viewport-control-test");
        app.inner_mut()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(PrimaryOutputState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    apply_output_overlay_controls_system,
                    remember_output_viewports_system,
                )
                    .chain(),
            );

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Virtual".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let output_id =
            *app.inner().world().get::<OutputId>(output).expect("output id should exist");
        app.inner_mut().world_mut().resource_mut::<PrimaryOutputState>().id = Some(output_id);
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(output_id);

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingOutputControls>()
            .named(OutputName::from("Virtual-1"))
            .move_viewport_to(100, 200)
            .pan_viewport_by(25, -40);

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let Some(viewport) = app.inner().world().get::<OutputViewport>(output) else {
            panic!("output viewport");
        };
        assert_eq!((viewport.origin_x, viewport.origin_y), (125, 160));
        let remembered = app.inner().world().resource::<RememberedOutputViewportState>();
        assert_eq!(
            remembered.viewport_for_output_name("Virtual-1"),
            Some(&OutputViewport { origin_x: 125, origin_y: 160 }),
        );
    }

    #[test]
    fn center_viewport_control_targets_window_scene_geometry() {
        let mut app = NekolandApp::new("output-viewport-center-test");
        app.inner_mut()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(PrimaryOutputState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    apply_output_overlay_controls_system,
                    remember_output_viewports_system,
                )
                    .chain(),
            );

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Virtual".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 800,
                    height: 600,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let output_id =
            *app.inner().world().get::<OutputId>(output).expect("output id should exist");
        app.inner_mut().world_mut().resource_mut::<PrimaryOutputState>().id = Some(output_id);
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(output_id);
        app.inner_mut().world_mut().spawn(WindowBundle {
            surface: WlSurfaceHandle { id: 77 },
            scene_geometry: WindowSceneGeometry { x: 1400, y: 900, width: 300, height: 200 },
            window: XdgWindow::default(),
            layout: WindowLayout::Floating,
            mode: WindowMode::Normal,
            ..Default::default()
        });

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingOutputControls>()
            .select(OutputSelector::Focused)
            .center_viewport_on_window(SurfaceId(77));

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let Some(viewport) = app.inner().world().get::<OutputViewport>(output) else {
            panic!("output viewport");
        };
        assert_eq!((viewport.origin_x, viewport.origin_y), (1150, 700));
    }

    #[test]
    fn overlay_controls_update_output_overlay_state() {
        let mut app = NekolandApp::new("output-overlay-control-test");
        app.inner_mut()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(PrimaryOutputState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    apply_output_overlay_controls_system,
                    remember_output_viewports_system,
                )
                    .chain(),
            );

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-1".to_owned(),
                    kind: OutputKind::Virtual,
                    make: "Virtual".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1280,
                    height: 720,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                ..Default::default()
            })
            .id();
        let output_id =
            *app.inner().world().get::<OutputId>(output).expect("output id should exist");

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingOutputControls>()
            .named(OutputName::from("Virtual-1"))
            .set_overlay_rect(
                "debug",
                RenderRect { x: 10, y: 20, width: 300, height: 200 },
                RenderColor { r: 1, g: 2, b: 3, a: 255 },
                Some(0.5),
                Some(9),
                Some(RenderRect { x: 30, y: 40, width: 50, height: 60 }),
            );

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let overlays = app.inner().world().resource::<OutputOverlayState>();
        let output = overlays.outputs.get(&output_id).expect("overlay state should exist");
        let overlay =
            output.overlays.get(&OutputOverlayId::from("debug")).expect("overlay should exist");
        assert_eq!(overlay.rect.width, 300);
        assert_eq!(overlay.clip_rect.map(|rect| rect.x), Some(30));
        assert_eq!(overlay.opacity, 0.5);
        assert_eq!(overlay.z_index, 9);
    }

    #[test]
    fn backend_reconnect_restores_remembered_output_viewport() {
        let mut app = NekolandApp::new("backend-output-reconnect-viewport-test");
        app.inner_mut()
            .insert_resource(BackendOutputRegistry::default())
            .insert_resource({
                let mut remembered = RememberedOutputViewportState::default();
                remembered.ids_by_name.insert("Virtual-1".to_owned(), OutputId(1));
                remembered
                    .by_id
                    .insert(OutputId(1), OutputViewport { origin_x: 640, origin_y: -320 });
                remembered
            })
            .init_resource::<PendingBackendOutputEvents>()
            .init_resource::<Messages<OutputConnected>>()
            .init_resource::<Messages<OutputDisconnected>>()
            .add_systems(ExtractSchedule, synchronize_backend_outputs_system);

        app.inner_mut().world_mut().resource_mut::<PendingBackendOutputEvents>().push(
            BackendOutputEventRecord {
                backend_id: BackendId(7),
                output_name: "Virtual-1".to_owned(),
                local_id: "virtual-primary".to_owned(),
                change: BackendOutputChange::Connected(BackendOutputBlueprint {
                    local_id: "virtual-primary".to_owned(),
                    device: OutputDevice {
                        name: "Virtual-1".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "Virtual".to_owned(),
                        model: "test".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1920,
                        height: 1080,
                        refresh_millihz: 60_000,
                        scale: 1,
                    },
                }),
            },
        );

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let world = app.inner_mut().world_mut();
        let output_state = world
            .query::<(&OutputDevice, &OutputViewport, &OutputBackend)>()
            .iter(world)
            .find(|(output, _, _)| output.name == "Virtual-1")
            .map(|(_, viewport, owner)| (viewport.clone(), owner.clone()));
        let Some((viewport, owner)) = output_state else {
            panic!("reconnected output should be materialized");
        };
        assert_eq!(viewport.origin_x, 640);
        assert_eq!(viewport.origin_y, -320);
        assert_eq!(owner.backend_id, BackendId(7));
    }
}
