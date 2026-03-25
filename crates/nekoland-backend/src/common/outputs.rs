use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::entity::Entity;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, NonSend, Query, Res, ResMut, Resource, With};
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::{CompositorConfig, ConfiguredOutput};
use nekoland_core::error::NekolandError;
use nekoland_ecs::bundles::OutputBundle;
use nekoland_ecs::components::{
    OutputDevice, OutputId, OutputPlacement, OutputProperties, OutputViewport, OutputWorkArea,
    WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::kinds::{BackendEvent, FrameQueue};
use nekoland_ecs::resources::{
    BackendOutputRegistry, CompositorClock, EntityIndex, FocusedOutputState,
    OutputGeometrySnapshot, OutputOverlayState, OutputServerAction, OutputServerRequest,
    OutputSnapshotState, OutputViewportAnimation, OutputViewportAnimationState,
    PendingOutputControl, PendingOutputControls, PendingOutputOverlayControls,
    PendingOutputServerRequests, PlatformOutputBlueprint, PlatformOutputLifecycleChange,
    PlatformOutputLifecycleRecord, PlatformOutputMaterializationPlan, PlatformOutputPropertyUpdate,
    ViewportAnimationActivityState, WaylandIngress,
};
use nekoland_ecs::selectors::OutputSelector;
use nekoland_ecs::views::OutputRuntime;
use serde::{Deserialize, Serialize};

use crate::components::OutputBackend;
use crate::manager::SharedBackendManager;
use crate::plugin::BackendPresentInputs;
use crate::traits::{BackendId, BackendOutputId, OutputSnapshot};

const OUTPUT_VIEWPORT_ANIMATION_DURATION_MS: u32 = 180;

/// Remembers output-local viewport origins across output disable/enable and reconnect cycles.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct RememberedOutputViewportState {
    pub by_id: BTreeMap<OutputId, OutputViewport>,
    pub by_name: BTreeMap<String, OutputViewport>,
    pub ids_by_name: BTreeMap<String, OutputId>,
}

impl RememberedOutputViewportState {
    pub fn viewport_for_output_id(&self, output_id: OutputId) -> Option<&OutputViewport> {
        self.by_id.get(&output_id)
    }

    pub fn viewport_for_output_name(&self, output_name: &str) -> Option<&OutputViewport> {
        self.by_name.get(output_name).or_else(|| {
            self.ids_by_name.get(output_name).and_then(|output_id| self.by_id.get(output_id))
        })
    }

    pub fn remember(&mut self, output_id: OutputId, output_name: String, viewport: OutputViewport) {
        self.ids_by_name.insert(output_name.clone(), output_id);
        self.by_name.insert(output_name, viewport.clone());
        self.by_id.insert(output_id, viewport);
    }

    pub fn forget_name(&mut self, output_name: &str) {
        if let Some(output_id) = self.ids_by_name.remove(output_name) {
            self.by_id.remove(&output_id);
        }
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

/// Stable output-materialization plan exported from the wayland subapp to the main world.
///
/// Unlike the raw backend extract queues, this plan is an ordinary snapshot payload with no
/// frame-queue behavior. The wayland subapp owns the raw queues; the main world only applies the
/// already-normalized materialization operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputMaterializationPlan {
    pub lifecycle: Vec<BackendOutputEventRecord>,
    pub property_updates: Vec<BackendOutputPropertyUpdate>,
}

impl BackendOutputMaterializationPlan {
    pub fn from_pending_queues(
        pending_output_events: &PendingBackendOutputEvents,
        pending_output_updates: &PendingBackendOutputUpdates,
    ) -> Self {
        Self {
            lifecycle: pending_output_events.as_slice().to_vec(),
            property_updates: pending_output_updates.as_slice().to_vec(),
        }
    }
}

impl From<BackendOutputBlueprint> for PlatformOutputBlueprint {
    fn from(value: BackendOutputBlueprint) -> Self {
        Self { device: value.device, properties: value.properties }
    }
}

impl From<BackendOutputChange> for PlatformOutputLifecycleChange {
    fn from(value: BackendOutputChange) -> Self {
        match value {
            BackendOutputChange::Connected(blueprint) => Self::Connected(blueprint.into()),
            BackendOutputChange::Disconnected => Self::Disconnected,
        }
    }
}

impl From<BackendOutputEventRecord> for PlatformOutputLifecycleRecord {
    fn from(value: BackendOutputEventRecord) -> Self {
        Self {
            backend_id: value.backend_id.0,
            output_name: value.output_name,
            local_id: value.local_id,
            change: value.change.into(),
        }
    }
}

impl From<BackendOutputPropertyUpdate> for PlatformOutputPropertyUpdate {
    fn from(value: BackendOutputPropertyUpdate) -> Self {
        Self {
            backend_id: value.backend_id.0,
            output_name: value.output_name,
            local_id: value.local_id,
            properties: value.properties,
        }
    }
}

impl From<BackendOutputMaterializationPlan> for PlatformOutputMaterializationPlan {
    fn from(value: BackendOutputMaterializationPlan) -> Self {
        Self {
            lifecycle: value.lifecycle.into_iter().map(Into::into).collect(),
            property_updates: value.property_updates.into_iter().map(Into::into).collect(),
        }
    }
}

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

#[derive(SystemParam)]
pub(crate) struct OutputControlRequestCtx<'w, 's> {
    clock: Option<Res<'w, CompositorClock>>,
    pending_output_controls: ResMut<'w, PendingOutputControls>,
    pending_output_overlay_controls: ResMut<'w, PendingOutputOverlayControls>,
    pending_output_requests: ResMut<'w, PendingOutputServerRequests>,
    wayland_ingress: Res<'w, WaylandIngress>,
    focused_output: Res<'w, FocusedOutputState>,
    entity_index: Res<'w, EntityIndex>,
    viewport_animations: ResMut<'w, OutputViewportAnimationState>,
    remembered_viewports: ResMut<'w, RememberedOutputViewportState>,
    outputs: OutputViewportQuery<'w, 's>,
    windows: OutputWindowSceneQuery<'w, 's>,
}

#[derive(SystemParam)]
pub(crate) struct OutputServerRequestCtx<'w, 's> {
    manager: NonSend<'w, SharedBackendManager>,
    outputs: Res<'w, BackendPresentInputs>,
    pending_output_requests: ResMut<'w, PendingOutputServerRequests>,
    pending_output_events: ResMut<'w, PendingBackendOutputEvents>,
    pending_output_updates: ResMut<'w, PendingBackendOutputUpdates>,
    _marker: std::marker::PhantomData<&'s ()>,
}

/// Translates the latest config snapshot into idempotent enable/configure/disable requests for
/// the backend-facing output controller.
pub fn sync_configured_outputs_system(
    config: Option<Res<CompositorConfig>>,
    outputs: Res<'_, BackendPresentInputs>,
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
        .outputs()
        .iter()
        .map(|output| (output.device.name.clone(), output.properties.clone()))
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
        clock,
        mut pending_output_controls,
        mut pending_output_overlay_controls,
        mut pending_output_requests,
        wayland_ingress,
        focused_output,
        entity_index,
        mut viewport_animations,
        mut remembered_viewports,
        mut outputs,
        windows,
    } = ctx;
    let current_uptime_millis = clock.as_deref().map_or(0, |clock| clock.uptime_millis);
    let primary_output_id = wayland_ingress.primary_output.id;
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
                let Some(output) = primary_output_id.and_then(|output_id| {
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
                        primary_output_id.and_then(|output_id| {
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

        let current_viewport =
            viewport_animations.sampled_viewport(*output_id, &viewport, current_uptime_millis);
        *viewport = current_viewport.clone();

        let mut staged_viewport = current_viewport.clone();
        let mut animated_target = None;
        if let Some(origin) = deferred_control.viewport_origin.take() {
            staged_viewport.move_to(origin.x, origin.y);
            animated_target = Some(staged_viewport.clone());
        }
        if let Some(pan) = deferred_control.viewport_pan.take() {
            staged_viewport.pan_by(pan.delta_x, pan.delta_y);
            if animated_target.is_some() {
                animated_target = Some(staged_viewport.clone());
            }
        }
        if let Some(surface_id) = deferred_control.center_viewport_on.take() {
            let target_window = entity_index
                .entity_for_surface(surface_id.0)
                .and_then(|entity| windows.get(entity).ok())
                .or_else(|| windows.iter().find(|(surface, _)| surface.id == surface_id.0));
            if let Some((_, scene_geometry)) = target_window {
                center_viewport_on_scene_geometry(
                    &mut staged_viewport,
                    scene_geometry,
                    output_properties,
                );
                animated_target = Some(staged_viewport.clone());
            } else {
                deferred_control.center_viewport_on = Some(surface_id);
            }
        }

        if let Some(target_viewport) = animated_target {
            if target_viewport == current_viewport {
                viewport_animations.cancel(*output_id);
                *viewport = target_viewport;
            } else {
                viewport_animations.start(
                    *output_id,
                    OutputViewportAnimation {
                        from: current_viewport.clone(),
                        to: target_viewport,
                        start_uptime_millis: current_uptime_millis,
                        duration_millis: OUTPUT_VIEWPORT_ANIMATION_DURATION_MS,
                    },
                );
                *viewport = current_viewport;
            }
        } else if staged_viewport != current_viewport {
            viewport_animations.cancel(*output_id);
            *viewport = staged_viewport;
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

/// Advances authoritative viewport animations before shell layout consumes output viewport state.
pub(crate) fn advance_output_viewport_animations_system(
    clock: Option<Res<'_, CompositorClock>>,
    mut animations: ResMut<'_, OutputViewportAnimationState>,
    mut activity: ResMut<'_, ViewportAnimationActivityState>,
    mut outputs: Query<'_, '_, (&OutputId, &'static mut OutputViewport)>,
) {
    let current_uptime_millis = clock.as_deref().map_or(0, |clock| clock.uptime_millis);
    let output_ids = animations.output_ids();
    let mut completed = Vec::new();
    let mut active_outputs = BTreeSet::new();

    for output_id in output_ids {
        let Some((_, mut viewport)) =
            outputs.iter_mut().find(|(candidate_id, _)| **candidate_id == output_id)
        else {
            completed.push(output_id);
            continue;
        };
        let Some(animation) = animations.animation_for(output_id).cloned() else {
            continue;
        };

        *viewport = animation.sample(current_uptime_millis);
        if animation.is_complete(current_uptime_millis) {
            *viewport = animation.to.clone();
            completed.push(output_id);
        } else {
            active_outputs.insert(output_id);
        }
    }

    for output_id in completed {
        animations.cancel(output_id);
    }
    activity.active_outputs = active_outputs;
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
    mut wayland_ingress: ResMut<WaylandIngress>,
    existing_outputs: Query<(Entity, &OutputId, &OutputDevice, &OutputBackend)>,
    mut output_properties: Query<(&OutputBackend, &mut OutputProperties)>,
    mut output_connected: MessageWriter<OutputConnected>,
    mut output_disconnected: MessageWriter<OutputDisconnected>,
) {
    let mut output_materialization = std::mem::take(&mut wayland_ingress.output_materialization);

    for (_, output_id, output, _) in &existing_outputs {
        output_registry.remember_connected(*output_id, output.name.clone());
        output_registry.remember_enabled(*output_id, output.name.clone());
    }

    for record in output_materialization.lifecycle.drain(..) {
        match record.change {
            PlatformOutputLifecycleChange::Connected(blueprint) => {
                if let Some((entity, output_id, _, _)) =
                    existing_outputs.iter().find(|(_, _, _, owner)| {
                        owner.output_id
                            == (BackendOutputId {
                                backend_id: BackendId(record.backend_id),
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
                        backend_id: BackendId(record.backend_id),
                        output_id: BackendOutputId {
                            backend_id: BackendId(record.backend_id),
                            local_id: record.local_id.clone(),
                        },
                    },
                ));
                output_registry.remember_connected(output_id, record.output_name.clone());
                output_registry.remember_enabled(output_id, record.output_name.clone());
                output_connected.write(OutputConnected { name: record.output_name.clone() });
            }
            PlatformOutputLifecycleChange::Disconnected => {
                for (entity, _, output, owner) in &existing_outputs {
                    if owner.output_id
                        == (BackendOutputId {
                            backend_id: BackendId(record.backend_id),
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

    for update in output_materialization.property_updates.drain(..) {
        for (owner, mut properties) in &mut output_properties {
            if owner.output_id
                == (BackendOutputId {
                    backend_id: BackendId(update.backend_id),
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
        manager,
        outputs,
        mut pending_output_requests,
        mut pending_output_events,
        mut pending_output_updates,
        ..
    } = ctx;
    let mut deferred = Vec::new();

    for request in pending_output_requests.drain() {
        match request.action {
            OutputServerAction::Enable { output } => {
                if outputs.outputs().iter().any(|existing| existing.device.name == output)
                    || pending_output_events.as_slice().iter().any(|record| {
                        record.output_name == output
                            && matches!(record.change, BackendOutputChange::Connected(_))
                    })
                {
                    continue;
                }

                let Some(seed) = manager.borrow().seed_output(&output) else {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Enable { output },
                    });
                    continue;
                };
                let crate::manager::SeededBackendOutput { backend_id, blueprint } = seed;
                let local_id = blueprint.local_id.clone();
                pending_output_events.push(BackendOutputEventRecord {
                    backend_id,
                    output_name: output,
                    local_id,
                    change: BackendOutputChange::Connected(blueprint),
                });
            }
            OutputServerAction::Disable { output } => {
                let Some(existing) =
                    outputs.outputs().iter().find(|existing| existing.device.name == output)
                else {
                    continue;
                };
                let Some(backend_output_id) = existing.backend_output_id.as_ref() else {
                    continue;
                };
                pending_output_events.push(BackendOutputEventRecord {
                    backend_id: backend_output_id.backend_id,
                    output_name: output,
                    local_id: backend_output_id.local_id.clone(),
                    change: BackendOutputChange::Disconnected,
                });
            }
            OutputServerAction::Configure { output, mode, scale } => {
                let Some(configured_mode) = parse_output_mode(&mode) else {
                    tracing::warn!(output, mode, "ignoring invalid output mode request");
                    continue;
                };

                let Some(existing) =
                    outputs.outputs().iter().find(|existing| existing.device.name == output)
                else {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Configure { output, mode, scale },
                    });
                    continue;
                };
                let Some(backend_output_id) = existing.backend_output_id.as_ref() else {
                    continue;
                };
                let mut properties = existing.properties.clone();

                properties.width = configured_mode.width;
                properties.height = configured_mode.height;
                properties.refresh_millihz = configured_mode.refresh_millihz;
                if let Some(scale) = scale {
                    properties.scale = scale.max(1);
                }

                pending_output_updates.push(BackendOutputPropertyUpdate {
                    backend_id: backend_output_id.backend_id,
                    output_name: existing.device.name.clone(),
                    local_id: backend_output_id.local_id.clone(),
                    properties,
                });
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

pub fn sync_output_snapshot_state_from_present_inputs_system(
    outputs: Res<'_, BackendPresentInputs>,
    mut snapshots: ResMut<'_, OutputSnapshotState>,
) {
    snapshots.outputs = outputs
        .outputs()
        .iter()
        .map(|output| OutputGeometrySnapshot {
            output_id: output.output_id,
            name: output.device.name.clone(),
            x: 0,
            y: 0,
            width: output.properties.width,
            height: output.properties.height,
            scale: output.properties.scale,
            refresh_millihz: output.properties.refresh_millihz,
        })
        .collect();
}

pub fn collect_output_snapshots(
    outputs: &Query<(Entity, OutputRuntime, Option<&OutputBackend>)>,
) -> Vec<OutputSnapshot> {
    outputs
        .iter()
        .map(|(_, output, owner)| OutputSnapshot {
            output_id: output.id(),
            backend_id: owner.map(|owner| owner.backend_id),
            backend_output_id: owner.map(|owner| owner.output_id.clone()),
            device: output.device.clone(),
            properties: output.properties.clone(),
        })
        .collect()
}

pub fn sync_output_snapshot_state_system(
    outputs: Query<OutputRuntime>,
    mut snapshots: ResMut<'_, OutputSnapshotState>,
) {
    snapshots.outputs = outputs
        .iter()
        .map(|output| OutputGeometrySnapshot {
            output_id: output.id(),
            name: output.name().to_owned(),
            x: output.placement.x,
            y: output.placement.y,
            width: output.properties.width,
            height: output.properties.height,
            scale: output.properties.scale,
            refresh_millihz: output.properties.refresh_millihz,
        })
        .collect();
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
    use bevy_ecs::system::{IntoSystem, System};
    use bevy_ecs::world::World;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::ExtractSchedule;
    use nekoland_ecs::bundles::{OutputBundle, WindowBundle};
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputPlacement, OutputProperties, OutputViewport,
        WindowLayout, WindowMode, WindowSceneGeometry, WlSurfaceHandle, XdgWindow,
    };
    use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
    use nekoland_ecs::resources::{
        CompositorClock, EntityIndex, FocusedOutputState, OutputOverlayId, OutputOverlayState,
        OutputServerAction, OutputServerRequest, OutputSnapshotState, OutputViewportAnimationState,
        PendingOutputControls, PendingOutputOverlayControls, PendingOutputServerRequests,
        PlatformOutputBlueprint, PlatformOutputLifecycleChange, PlatformOutputLifecycleRecord,
        PlatformOutputMaterializationPlan, PlatformOutputPropertyUpdate, PrimaryOutputState,
        RenderColor, RenderRect, ViewportAnimationActivityState, WaylandIngress,
    };
    use nekoland_ecs::selectors::{OutputName, OutputSelector, SurfaceId};
    use nekoland_ecs::views::OutputRuntime;

    use crate::components::OutputBackend;
    use crate::manager::{BackendManager, SharedBackendManager};
    use crate::plugin::BackendPresentInputs;
    use crate::traits::BackendId;

    use super::{
        BackendOutputChange, BackendOutputRegistry, PendingBackendOutputEvents,
        PendingBackendOutputUpdates, RememberedOutputViewportState,
        advance_output_viewport_animations_system, apply_output_control_requests_system,
        apply_output_overlay_controls_system, apply_output_server_requests_system,
        collect_output_snapshots, remember_output_viewports_system, sync_configured_outputs_system,
        sync_output_snapshot_state_system, synchronize_backend_outputs_system,
    };

    #[derive(Debug, Default, bevy_ecs::prelude::Resource)]
    struct SnapshotAudit(Vec<crate::traits::OutputSnapshot>);

    #[derive(Debug, Default, bevy_ecs::prelude::Resource)]
    struct OutputGeometryAudit(OutputSnapshotState);

    #[test]
    fn viewport_controls_update_output_viewport_state() {
        let mut app = NekolandApp::new("output-viewport-control-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 0 })
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(OutputViewportAnimationState::default())
            .insert_resource(ViewportAnimationActivityState::default())
            .init_resource::<WaylandIngress>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    advance_output_viewport_animations_system,
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
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(output_id);
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(output_id);

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
        assert_eq!((viewport.origin_x, viewport.origin_y), (0, 0));
        assert!(
            app.inner()
                .world()
                .resource::<OutputViewportAnimationState>()
                .animation_for(output_id)
                .is_some()
        );

        app.inner_mut()
            .world_mut()
            .insert_resource(CompositorClock { frame: 2, uptime_millis: 180 });
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let Some(viewport) = app.inner().world().get::<OutputViewport>(output) else {
            panic!("output viewport");
        };
        assert_eq!((viewport.origin_x, viewport.origin_y), (125, 160));
        assert!(
            app.inner()
                .world()
                .resource::<OutputViewportAnimationState>()
                .animation_for(output_id)
                .is_none()
        );
        let remembered = app.inner().world().resource::<RememberedOutputViewportState>();
        assert_eq!(
            remembered.viewport_for_output_name("Virtual-1"),
            Some(&OutputViewport { origin_x: 125, origin_y: 160 }),
        );
    }

    #[test]
    fn primary_output_controls_prefer_wayland_ingress_over_stale_compat_state() {
        let mut app = NekolandApp::new("output-primary-control-ingress-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 0 })
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(OutputViewportAnimationState::default())
            .insert_resource(ViewportAnimationActivityState::default())
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(PrimaryOutputState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .insert_resource(WaylandIngress::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    advance_output_viewport_animations_system,
                    apply_output_overlay_controls_system,
                    remember_output_viewports_system,
                )
                    .chain(),
            );

        let output_one = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
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
            })
            .id();
        let output_two = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "Virtual-2".to_owned(),
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
            })
            .id();
        let output_one_id =
            *app.inner().world().get::<OutputId>(output_one).expect("first output id");
        let output_two_id =
            *app.inner().world().get::<OutputId>(output_two).expect("second output id");

        // Keep a stale compat primary-output snapshot on purpose so the selector must ignore it.
        app.inner_mut().world_mut().resource_mut::<PrimaryOutputState>().id = Some(output_one_id);
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(output_two_id);
        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingOutputControls>()
            .select(OutputSelector::Primary)
            .move_viewport_to(90, 140);

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);
        app.inner_mut()
            .world_mut()
            .insert_resource(CompositorClock { frame: 2, uptime_millis: 180 });
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let first_viewport =
            app.inner().world().get::<OutputViewport>(output_one).expect("first viewport");
        let second_viewport =
            app.inner().world().get::<OutputViewport>(output_two).expect("second viewport");
        assert_eq!((first_viewport.origin_x, first_viewport.origin_y), (0, 0));
        assert_eq!((second_viewport.origin_x, second_viewport.origin_y), (90, 140));
    }

    #[test]
    fn output_snapshots_are_normalized_without_entity_handles() {
        let mut app = NekolandApp::new("output-snapshot-normalization-test");
        app.inner_mut()
            .init_resource::<SnapshotAudit>()
            .add_systems(ExtractSchedule, capture_output_snapshots_system);
        let output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputBundle {
                    output: OutputDevice {
                        name: "Virtual-2".to_owned(),
                        kind: OutputKind::Virtual,
                        make: "Virtual".to_owned(),
                        model: "test".to_owned(),
                    },
                    properties: OutputProperties {
                        width: 1024,
                        height: 768,
                        refresh_millihz: 60_000,
                        scale: 2,
                    },
                    ..Default::default()
                },
                OutputBackend {
                    backend_id: BackendId(7),
                    output_id: crate::traits::BackendOutputId {
                        backend_id: BackendId(7),
                        local_id: "Virtual-2".to_owned(),
                    },
                },
            ))
            .id();
        let output_id = *app.inner().world().get::<OutputId>(output).expect("output id");
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let snapshots = &app.inner().world().resource::<SnapshotAudit>().0;
        assert_eq!(snapshots.len(), 1);
        let snapshot = &snapshots[0];
        assert_eq!(snapshot.output_id, output_id);
        assert_eq!(snapshot.backend_id, Some(BackendId(7)));
        assert_eq!(
            snapshot.backend_output_id.as_ref().map(|id| id.local_id.as_str()),
            Some("Virtual-2")
        );
        assert_eq!(snapshot.device.name, "Virtual-2");
        assert_eq!(snapshot.properties.scale, 2);
    }

    fn capture_output_snapshots_system(
        outputs: bevy_ecs::prelude::Query<
            '_,
            '_,
            (bevy_ecs::entity::Entity, OutputRuntime, Option<&'static OutputBackend>),
        >,
        mut audit: bevy_ecs::prelude::ResMut<'_, SnapshotAudit>,
    ) {
        audit.0 = collect_output_snapshots(&outputs);
    }

    fn capture_output_geometry_snapshots_system(
        snapshots: bevy_ecs::prelude::Res<'_, OutputSnapshotState>,
        mut audit: bevy_ecs::prelude::ResMut<'_, OutputGeometryAudit>,
    ) {
        audit.0 = snapshots.clone();
    }

    #[test]
    fn output_geometry_snapshot_state_is_normalized_without_runtime_handles() {
        let mut app = NekolandApp::new("output-geometry-snapshot-test");
        app.inner_mut()
            .init_resource::<OutputSnapshotState>()
            .init_resource::<OutputGeometryAudit>()
            .add_systems(
                ExtractSchedule,
                (sync_output_snapshot_state_system, capture_output_geometry_snapshots_system)
                    .chain(),
            );

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "DP-1".to_owned(),
                    kind: OutputKind::Nested,
                    make: "Nekoland".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1920,
                    height: 1080,
                    refresh_millihz: 60_000,
                    scale: 2,
                },
                placement: OutputPlacement { x: 200, y: 100 },
                ..Default::default()
            })
            .id();
        let output_id = *app.inner().world().get::<OutputId>(output).expect("output id");

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let snapshots = &app.inner().world().resource::<OutputGeometryAudit>().0.outputs;
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].output_id, output_id);
        assert_eq!(snapshots[0].name, "DP-1");
        assert_eq!(snapshots[0].x, 200);
        assert_eq!(snapshots[0].y, 100);
        assert_eq!(snapshots[0].width, 1920);
        assert_eq!(snapshots[0].height, 1080);
        assert_eq!(snapshots[0].scale, 2);
    }

    #[test]
    fn center_viewport_control_targets_window_scene_geometry() {
        let mut app = NekolandApp::new("output-viewport-center-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 0 })
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(OutputViewportAnimationState::default())
            .insert_resource(ViewportAnimationActivityState::default())
            .init_resource::<WaylandIngress>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    advance_output_viewport_animations_system,
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
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(output_id);
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(output_id);
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
        app.inner_mut()
            .world_mut()
            .insert_resource(CompositorClock { frame: 2, uptime_millis: 180 });
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let Some(viewport) = app.inner().world().get::<OutputViewport>(output) else {
            panic!("output viewport");
        };
        assert_eq!((viewport.origin_x, viewport.origin_y), (1150, 700));
    }

    #[test]
    fn viewport_pan_cancels_active_output_viewport_animation() {
        let mut app = NekolandApp::new("output-viewport-pan-cancel-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 0 })
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(OutputViewportAnimationState::default())
            .insert_resource(ViewportAnimationActivityState::default())
            .init_resource::<WaylandIngress>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    advance_output_viewport_animations_system,
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
        app.inner_mut().world_mut().resource_mut::<FocusedOutputState>().id = Some(output_id);
        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().primary_output.id =
            Some(output_id);

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingOutputControls>()
            .named(OutputName::from("Virtual-1"))
            .move_viewport_to(100, 200);

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);
        app.inner_mut()
            .world_mut()
            .insert_resource(CompositorClock { frame: 2, uptime_millis: 90 });
        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let viewport = app.inner().world().get::<OutputViewport>(output).expect("output viewport");
        assert_eq!((viewport.origin_x, viewport.origin_y), (50, 100));

        app.inner_mut()
            .world_mut()
            .resource_mut::<PendingOutputControls>()
            .named(OutputName::from("Virtual-1"))
            .pan_viewport_by(25, -40);

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let viewport = app.inner().world().get::<OutputViewport>(output).expect("output viewport");
        assert_eq!((viewport.origin_x, viewport.origin_y), (75, 60));
        assert!(
            app.inner()
                .world()
                .resource::<OutputViewportAnimationState>()
                .animation_for(output_id)
                .is_none()
        );
    }

    #[test]
    fn overlay_controls_update_output_overlay_state() {
        let mut app = NekolandApp::new("output-overlay-control-test");
        app.inner_mut()
            .insert_resource(CompositorClock { frame: 1, uptime_millis: 0 })
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingOutputOverlayControls>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<OutputOverlayState>()
            .insert_resource(OutputViewportAnimationState::default())
            .insert_resource(ViewportAnimationActivityState::default())
            .init_resource::<WaylandIngress>()
            .insert_resource(RememberedOutputViewportState::default())
            .insert_resource(FocusedOutputState::default())
            .insert_resource(EntityIndex::default())
            .add_systems(
                ExtractSchedule,
                (
                    apply_output_control_requests_system,
                    advance_output_viewport_animations_system,
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
            .insert_resource(RememberedOutputViewportState::default())
            .init_resource::<WaylandIngress>()
            .init_resource::<Messages<OutputConnected>>()
            .init_resource::<Messages<OutputDisconnected>>()
            .add_systems(
                ExtractSchedule,
                (remember_output_viewports_system, synchronize_backend_outputs_system).chain(),
            );

        app.inner_mut().world_mut().spawn((
            OutputId(1),
            OutputBundle {
                output: OutputDevice {
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
                viewport: OutputViewport { origin_x: 640, origin_y: -320 },
                ..Default::default()
            },
            OutputBackend {
                backend_id: BackendId(7),
                output_id: crate::traits::BackendOutputId {
                    backend_id: BackendId(7),
                    local_id: "virtual-primary".to_owned(),
                },
            },
        ));

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        app.inner_mut()
            .world_mut()
            .resource_mut::<WaylandIngress>()
            .output_materialization
            .lifecycle
            .push(PlatformOutputLifecycleRecord {
                backend_id: 7,
                output_name: "Virtual-1".to_owned(),
                local_id: "virtual-primary".to_owned(),
                change: PlatformOutputLifecycleChange::Disconnected,
            });

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        app.inner_mut()
            .world_mut()
            .resource_mut::<WaylandIngress>()
            .output_materialization
            .lifecycle
            .push(PlatformOutputLifecycleRecord {
                backend_id: 7,
                output_name: "Virtual-1".to_owned(),
                local_id: "virtual-primary".to_owned(),
                change: PlatformOutputLifecycleChange::Connected(PlatformOutputBlueprint {
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
            });

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
        assert_eq!(
            world.resource::<RememberedOutputViewportState>().viewport_for_output_name("Virtual-1"),
            Some(&OutputViewport { origin_x: 640, origin_y: -320 })
        );
    }

    #[test]
    fn backend_output_materialization_plan_updates_existing_output_properties() {
        let mut app = NekolandApp::new("backend-output-materialization-update-test");
        app.inner_mut()
            .insert_resource(BackendOutputRegistry::default())
            .insert_resource(RememberedOutputViewportState::default())
            .init_resource::<WaylandIngress>()
            .init_resource::<Messages<OutputConnected>>()
            .init_resource::<Messages<OutputDisconnected>>()
            .add_systems(ExtractSchedule, synchronize_backend_outputs_system);

        let output = app
            .inner_mut()
            .world_mut()
            .spawn((
                OutputId(1),
                OutputBundle {
                    output: OutputDevice {
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
                    ..Default::default()
                },
                OutputBackend {
                    backend_id: BackendId(7),
                    output_id: crate::traits::BackendOutputId {
                        backend_id: BackendId(7),
                        local_id: "virtual-primary".to_owned(),
                    },
                },
            ))
            .id();

        app.inner_mut().world_mut().resource_mut::<WaylandIngress>().output_materialization =
            PlatformOutputMaterializationPlan {
                property_updates: vec![PlatformOutputPropertyUpdate {
                    backend_id: 7,
                    output_name: "Virtual-1".to_owned(),
                    local_id: "virtual-primary".to_owned(),
                    properties: OutputProperties {
                        width: 2560,
                        height: 1440,
                        refresh_millihz: 59_940,
                        scale: 2,
                    },
                }],
                ..Default::default()
            };

        app.inner_mut().world_mut().run_schedule(ExtractSchedule);

        let properties =
            app.inner().world().get::<OutputProperties>(output).expect("output properties");
        assert_eq!(properties.width, 2560);
        assert_eq!(properties.height, 1440);
        assert_eq!(properties.refresh_millihz, 59_940);
        assert_eq!(properties.scale, 2);
    }

    #[test]
    fn output_server_disable_requests_emit_backend_disconnect_events() {
        let mut world = World::default();
        world.insert_non_send_resource(SharedBackendManager::new(BackendManager::default()));
        world.insert_resource(BackendPresentInputs::from_outputs(vec![
            crate::traits::OutputSnapshot {
                output_id: OutputId(7),
                backend_id: Some(BackendId(3)),
                backend_output_id: Some(crate::traits::BackendOutputId {
                    backend_id: BackendId(3),
                    local_id: "virtual-primary".to_owned(),
                }),
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
            },
        ]));
        let mut pending_requests = PendingOutputServerRequests::default();
        pending_requests.push(OutputServerRequest {
            action: OutputServerAction::Disable { output: "Virtual-1".to_owned() },
        });
        world.insert_resource(pending_requests);
        world.init_resource::<PendingBackendOutputEvents>();
        world.init_resource::<PendingBackendOutputUpdates>();

        let mut system = IntoSystem::into_system(apply_output_server_requests_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let events = world.resource::<PendingBackendOutputEvents>();
        assert_eq!(events.as_slice().len(), 1);
        let event = &events.as_slice()[0];
        assert_eq!(event.backend_id, BackendId(3));
        assert_eq!(event.output_name, "Virtual-1");
        assert_eq!(event.local_id, "virtual-primary");
        assert!(matches!(event.change, BackendOutputChange::Disconnected));
    }

    #[test]
    fn output_server_configure_requests_emit_backend_property_updates() {
        let mut world = World::default();
        world.insert_non_send_resource(SharedBackendManager::new(BackendManager::default()));
        world.insert_resource(BackendPresentInputs::from_outputs(vec![
            crate::traits::OutputSnapshot {
                output_id: OutputId(7),
                backend_id: Some(BackendId(3)),
                backend_output_id: Some(crate::traits::BackendOutputId {
                    backend_id: BackendId(3),
                    local_id: "virtual-primary".to_owned(),
                }),
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
            },
        ]));
        let mut pending_requests = PendingOutputServerRequests::default();
        pending_requests.push(OutputServerRequest {
            action: OutputServerAction::Configure {
                output: "Virtual-1".to_owned(),
                mode: "1280x720@60".to_owned(),
                scale: Some(2),
            },
        });
        world.insert_resource(pending_requests);
        world.init_resource::<PendingBackendOutputEvents>();
        world.init_resource::<PendingBackendOutputUpdates>();

        let mut system = IntoSystem::into_system(apply_output_server_requests_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let updates = world.resource::<PendingBackendOutputUpdates>();
        assert_eq!(updates.as_slice().len(), 1);
        let update = &updates.as_slice()[0];
        assert_eq!(update.backend_id, BackendId(3));
        assert_eq!(update.output_name, "Virtual-1");
        assert_eq!(update.local_id, "virtual-primary");
        assert_eq!(update.properties.width, 1280);
        assert_eq!(update.properties.height, 720);
        assert_eq!(update.properties.refresh_millihz, 60_000);
        assert_eq!(update.properties.scale, 2);
    }

    #[test]
    fn configured_outputs_sync_from_backend_present_input_snapshots() {
        let mut world = World::default();
        world.insert_resource(nekoland_config::resources::CompositorConfig {
            outputs: vec![nekoland_config::resources::ConfiguredOutput {
                name: "Virtual-2".to_owned(),
                enabled: true,
                mode: "1280x720@60".to_owned(),
                scale: 2,
            }],
            ..Default::default()
        });
        world.insert_resource(BackendPresentInputs::default());
        world.init_resource::<PendingOutputServerRequests>();

        let mut system = IntoSystem::into_system(sync_configured_outputs_system);
        system.initialize(&mut world);
        let _: Result<(), _> = system.run((), &mut world);

        let requests = world.resource::<PendingOutputServerRequests>();
        let actions =
            requests.as_slice().iter().map(|request| request.action.clone()).collect::<Vec<_>>();
        assert!(actions.iter().any(|action| matches!(
            action,
            OutputServerAction::Enable { output } if output == "Virtual-2"
        )));
        assert!(actions.iter().any(|action| matches!(
            action,
            OutputServerAction::Configure { output, mode, scale }
                if output == "Virtual-2" && mode == "1280x720@60" && *scale == Some(2)
        )));
    }
}
