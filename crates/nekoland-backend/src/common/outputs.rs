use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::entity::Entity;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Query, Res, ResMut, Resource};
use nekoland_core::error::NekolandError;
use nekoland_ecs::bundles::OutputBundle;
use nekoland_ecs::components::{OutputDevice, OutputProperties};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::kinds::{BackendEvent, FrameQueue};
use nekoland_ecs::resources::{
    CompositorConfig, ConfiguredOutput, OutputServerAction, OutputServerRequest,
    PendingOutputControls, PendingOutputServerRequests, PrimaryOutputState,
};
use nekoland_ecs::selectors::OutputSelector;
use nekoland_ecs::views::OutputRuntime;
use serde::{Deserialize, Serialize};

use crate::components::OutputBackend;
use crate::manager::BackendManager;
use crate::traits::{BackendId, OutputSnapshot};

/// Tracks the output names that are currently materialized as ECS entities.
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputRegistry {
    /// Output names currently materialized as ECS entities, sorted for stable queries.
    pub connected_outputs: Vec<String>,
}

/// Output metadata that a backend runtime wants the ECS world to materialize.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendOutputBlueprint {
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
    /// Stable output name used to match existing ECS entities.
    pub output_name: String,
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
    /// Stable output name used to find the ECS entity to update.
    pub output_name: String,
    /// Replacement output properties produced by the backend extract phase.
    pub properties: OutputProperties,
}

/// Per-frame queue of output property refreshes produced during backend extract.
pub type PendingBackendOutputUpdates =
    FrameQueue<BackendOutputPropertyUpdate, BackendOutputUpdateQueueTag>;

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
pub fn apply_output_control_requests_system(
    mut pending_output_controls: ResMut<PendingOutputControls>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
    primary_output: Res<PrimaryOutputState>,
) {
    let mut deferred = Vec::new();

    for control in pending_output_controls.take() {
        let output = match &control.selector {
            OutputSelector::Name(output) => output.as_str().to_owned(),
            OutputSelector::Primary => {
                let Some(output) = primary_output.name.clone() else {
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
                    output,
                    mode: configuration.mode,
                    scale: configuration.scale,
                },
            });
        }
    }

    pending_output_controls.replace(deferred);
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
    mut pending_output_events: ResMut<PendingBackendOutputEvents>,
    existing_outputs: Query<(Entity, &OutputDevice, &OutputBackend)>,
    mut output_connected: MessageWriter<OutputConnected>,
    mut output_disconnected: MessageWriter<OutputDisconnected>,
) {
    let mut known_outputs =
        existing_outputs.iter().map(|(_, output, _)| output.name.clone()).collect::<BTreeSet<_>>();

    for record in pending_output_events.drain() {
        match record.change {
            BackendOutputChange::Connected(blueprint) => {
                if let Some((entity, _, _)) = existing_outputs.iter().find(|(_, output, owner)| {
                    output.name == record.output_name && owner.backend_id == record.backend_id
                }) {
                    commands
                        .entity(entity)
                        .insert((blueprint.device.clone(), blueprint.properties));
                    continue;
                }

                commands.spawn((
                    OutputBundle { output: blueprint.device, properties: blueprint.properties },
                    OutputBackend { backend_id: record.backend_id },
                ));
                output_connected.write(OutputConnected { name: record.output_name.clone() });
                known_outputs.insert(record.output_name);
            }
            BackendOutputChange::Disconnected => {
                for (entity, output, owner) in &existing_outputs {
                    if output.name == record.output_name && owner.backend_id == record.backend_id {
                        commands.entity(entity).despawn();
                    }
                }
                output_disconnected.write(OutputDisconnected { name: record.output_name.clone() });
                known_outputs.remove(&record.output_name);
            }
        }
    }

    output_registry.connected_outputs = known_outputs.into_iter().collect();
}

/// Applies backend-originated property refreshes to already-materialized ECS output entities.
pub fn apply_backend_output_updates_system(
    mut outputs: Query<(&OutputDevice, &OutputBackend, &mut OutputProperties)>,
    mut pending_updates: ResMut<PendingBackendOutputUpdates>,
) {
    for update in pending_updates.drain() {
        for (output, owner, mut properties) in &mut outputs {
            if output.name == update.output_name && owner.backend_id == update.backend_id {
                *properties = update.properties.clone();
                break;
            }
        }
    }
}

/// Applies public output-management requests against ECS output state and explicit backend
/// ownership metadata.
/// Materialize public output-management requests against currently installed
/// backends, deferring requests that cannot yet be satisfied this frame.
pub fn apply_output_server_requests_system(
    mut commands: Commands,
    manager: bevy_ecs::prelude::NonSend<BackendManager>,
    mut output_registry: ResMut<BackendOutputRegistry>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
    mut outputs: Query<(Entity, &OutputDevice, &OutputBackend, &mut OutputProperties)>,
    mut output_connected: MessageWriter<OutputConnected>,
    mut output_disconnected: MessageWriter<OutputDisconnected>,
) {
    let mut deferred = Vec::new();

    for request in pending_output_requests.drain() {
        match request.action {
            OutputServerAction::Enable { output } => {
                if outputs.iter().any(|(_, existing, _, _)| existing.name == output) {
                    continue;
                }

                let Some(seed) = manager.seed_output(&output) else {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Enable { output },
                    });
                    continue;
                };

                commands.spawn((
                    OutputBundle {
                        output: seed.blueprint.device,
                        properties: seed.blueprint.properties,
                    },
                    OutputBackend { backend_id: seed.backend_id },
                ));
                if !output_registry.connected_outputs.iter().any(|name| name == &output) {
                    output_registry.connected_outputs.push(output.clone());
                    output_registry.connected_outputs.sort();
                }
                output_connected.write(OutputConnected { name: output });
            }
            OutputServerAction::Disable { output } => {
                let Some((entity, _, _, _)) =
                    outputs.iter_mut().find(|(_, existing, _, _)| existing.name == output)
                else {
                    continue;
                };

                commands.entity(entity).despawn();
                output_registry.connected_outputs.retain(|name| name != &output);
                output_disconnected.write(OutputDisconnected { name: output });
            }
            OutputServerAction::Configure { output, mode, scale } => {
                let Some(configured_mode) = parse_output_mode(&mode) else {
                    tracing::warn!(output, mode, "ignoring invalid output mode request");
                    continue;
                };

                let Some((_, _, _, mut properties)) =
                    outputs.iter_mut().find(|(_, existing, _, _)| existing.name == output)
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

/// Refresh the public primary-output snapshot from current outputs and config preference order.
pub fn sync_primary_output_state_system(
    config: Option<Res<CompositorConfig>>,
    outputs: Query<OutputRuntime>,
    mut primary_output: ResMut<PrimaryOutputState>,
) {
    let next_primary = select_primary_output_name(config.as_deref(), &outputs);
    if primary_output.name != next_primary {
        tracing::trace!(previous = ?primary_output.name, next = ?next_primary, "updated primary output");
        primary_output.name = next_primary;
    }
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
            backend_id: owner.map(|owner| owner.backend_id),
            device: output.device.clone(),
            properties: output.properties.clone(),
        })
        .collect()
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
