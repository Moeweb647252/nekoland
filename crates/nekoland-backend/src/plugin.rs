use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bevy_app::App;
use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Commands, Entity, Query, Res, ResMut, Resource};
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{ExtractSchedule, PresentSchedule};
use nekoland_ecs::bundles::OutputBundle;
use nekoland_ecs::components::{OutputDevice, OutputKind, OutputProperties};
use nekoland_ecs::events::{OutputConnected, OutputDisconnected};
use nekoland_ecs::resources::{
    CompositorConfig, ConfiguredOutput, OutputEventRecord, OutputPresentationEventRecord,
    OutputPresentationState, OutputPresentationTimeline, OutputServerAction, OutputServerRequest,
    PendingBackendInputEvents, PendingOutputEvents, PendingOutputPresentationEvents,
    PendingOutputServerRequests, PendingProtocolInputEvents, VirtualOutputCaptureState,
};
use smithay::utils::{Clock, Monotonic};

use crate::{
    drm,
    traits::{BackendKind, SelectedBackend},
    virtual_output, winit,
};

use drm::device::SharedDrmState;
use drm::gbm::SharedGbmState;
use drm::surface::DrmRenderState;

#[derive(Debug, Clone, Default, Resource)]
pub struct BackendOutputRegistry {
    pub connected_outputs: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct OutputPresentationRuntime {
    outputs: BTreeMap<String, OutputPresentationClock>,
}

#[derive(Debug, Clone, Copy)]
struct OutputPresentationClock {
    anchor_nanos: u64,
    refresh_interval_nanos: u64,
    sequence: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BackendPlugin;

impl NekolandPlugin for BackendPlugin {
    fn build(&self, app: &mut App) {
        winit::backend::install_winit_present_completion_source(app);

        app.insert_resource(SelectedBackend::default())
            .insert_resource(BackendOutputRegistry::default())
            .init_resource::<VirtualOutputCaptureState>()
            .init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingProtocolInputEvents>()
            .init_resource::<PendingOutputEvents>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<PendingOutputPresentationEvents>()
            .init_resource::<OutputPresentationState>()
            // DRM backend shared state (NonSend: raw fd must stay on main thread).
            .insert_non_send_resource(SharedDrmState::default())
            .insert_non_send_resource(SharedGbmState::default())
            .insert_non_send_resource(DrmRenderState::default())
            .add_message::<OutputConnected>()
            .add_message::<OutputDisconnected>()
            .add_systems(
                ExtractSchedule,
                (
                    detect_backend_system,
                    virtual_output::virtual_backend_system,
                    drm::device::drm_device_system,
                    drm::surface::drm_present_completion_system,
                    drm::gbm::gbm_allocator_system,
                    winit::backend::winit_backend_system,
                    ensure_primary_output_system,
                    synchronize_outputs_system,
                    sync_configured_outputs_system,
                    apply_output_server_requests_system,
                    winit::backend::sync_winit_window_system,
                    virtual_output::virtual_present_completion_system,
                    winit::backend::winit_present_completion_system,
                    apply_output_presentation_events_system,
                )
                    .chain(),
            )
            .add_systems(
                PresentSchedule,
                (
                    virtual_output::virtual_output_capture_system,
                    // DRM and Winit render systems are mutually exclusive at
                    // runtime — each is gated by SelectedBackend.
                    drm::surface::drm_render_system,
                    winit::backend::winit_render_system,
                )
                    .chain(),
            );
    }
}

pub fn detect_backend_system(mut selected_backend: ResMut<SelectedBackend>) {
    if selected_backend.kind != BackendKind::Auto {
        return;
    }

    let env_backend = std::env::var("NEKOLAND_BACKEND").unwrap_or_else(|_| "winit".to_owned());
    selected_backend.kind = match env_backend.as_str() {
        "drm" => BackendKind::Drm,
        "winit" => BackendKind::Winit,
        "virtual" | "headless" | "offscreen" => BackendKind::Virtual,
        _ => BackendKind::Winit,
    };
    selected_backend.description = format!("selected via NEKOLAND_BACKEND={env_backend}");

    if env_backend == "x11" {
        tracing::warn!("NEKOLAND_BACKEND=x11 is deprecated; using winit backend");
    }

    tracing::info!(kind = ?selected_backend.kind, "backend selected");
}

fn ensure_primary_output_system(
    selected_backend: Res<SelectedBackend>,
    config: Option<Res<CompositorConfig>>,
    existing_outputs: Query<&OutputDevice>,
    mut pending_output_events: ResMut<PendingOutputEvents>,
    mut seeded: bevy_ecs::prelude::Local<bool>,
) {
    if *seeded || selected_backend.kind == BackendKind::Auto || !existing_outputs.is_empty() {
        return;
    }

    let output_name = if let Some(config) = config.as_deref() {
        if config.outputs.is_empty() {
            default_output_name(selected_backend.kind.clone())
        } else {
            config.outputs.iter().find(|output| output.enabled).map(|output| output.name.as_str())
        }
    } else {
        default_output_name(selected_backend.kind.clone())
    };
    let Some(output_name) = output_name else {
        *seeded = true;
        return;
    };

    if pending_output_events.items.iter().any(|record| {
        record.output_name == output_name
            && matches!(record.change.as_str(), "announced" | "connected")
    }) {
        return;
    }

    pending_output_events.items.push(OutputEventRecord {
        output_name: output_name.to_owned(),
        change: "connected".to_owned(),
    });
    *seeded = true;
}

fn sync_configured_outputs_system(
    config: Option<Res<CompositorConfig>>,
    outputs: Query<(&OutputDevice, &OutputProperties)>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
    mut last_applied_outputs: bevy_ecs::prelude::Local<Option<Vec<ConfiguredOutput>>>,
) {
    let Some(config) = config else {
        return;
    };

    if last_applied_outputs.as_ref() == Some(&config.outputs) {
        return;
    }

    if config.outputs.is_empty() {
        *last_applied_outputs = Some(Vec::new());
        return;
    }

    let existing_outputs = outputs
        .iter()
        .map(|(output, properties)| (output.name.clone(), properties.clone()))
        .collect::<BTreeMap<_, _>>();

    for configured_output in &config.outputs {
        if configured_output.enabled {
            if !existing_outputs.contains_key(&configured_output.name) {
                pending_output_requests.items.push(OutputServerRequest {
                    action: OutputServerAction::Enable { output: configured_output.name.clone() },
                });
            }

            if existing_outputs
                .get(&configured_output.name)
                .is_none_or(|properties| !output_matches_config(properties, configured_output))
            {
                pending_output_requests.items.push(OutputServerRequest {
                    action: OutputServerAction::Configure {
                        output: configured_output.name.clone(),
                        mode: configured_output.mode.clone(),
                        scale: Some(configured_output.scale.max(1)),
                    },
                });
            }
        } else if existing_outputs.contains_key(&configured_output.name) {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Disable { output: configured_output.name.clone() },
            });
        }
    }

    let configured_names =
        config.outputs.iter().map(|output| output.name.as_str()).collect::<BTreeSet<_>>();
    for output_name in existing_outputs.keys() {
        if !configured_names.contains(output_name.as_str()) {
            pending_output_requests.items.push(OutputServerRequest {
                action: OutputServerAction::Disable { output: output_name.clone() },
            });
        }
    }

    *last_applied_outputs = Some(config.outputs.clone());
}

fn synchronize_outputs_system(
    mut commands: Commands,
    selected_backend: Res<SelectedBackend>,
    mut output_registry: ResMut<BackendOutputRegistry>,
    mut pending_output_events: ResMut<PendingOutputEvents>,
    existing_outputs: Query<(Entity, &OutputDevice)>,
    mut output_connected: MessageWriter<OutputConnected>,
    mut output_disconnected: MessageWriter<OutputDisconnected>,
) {
    let mut known_outputs =
        existing_outputs.iter().map(|(_, output)| output.name.clone()).collect::<BTreeSet<_>>();

    for record in pending_output_events.items.drain(..) {
        match record.change.as_str() {
            "announced" | "connected" => {
                if known_outputs.insert(record.output_name.clone()) {
                    let properties = match selected_backend.kind {
                        BackendKind::Drm => OutputProperties {
                            width: 2560,
                            height: 1440,
                            refresh_millihz: 144_000,
                            scale: 1,
                        },
                        BackendKind::Virtual => OutputProperties {
                            width: 1920,
                            height: 1080,
                            refresh_millihz: 60_000,
                            scale: 1,
                        },
                        BackendKind::Winit | BackendKind::Auto => OutputProperties {
                            width: 1280,
                            height: 720,
                            refresh_millihz: 60_000,
                            scale: 1,
                        },
                    };
                    commands.spawn(OutputBundle {
                        output: OutputDevice {
                            name: record.output_name.clone(),
                            kind: output_kind_for_backend(selected_backend.kind.clone()),
                            make: format!("{:?}", selected_backend.kind),
                            model: selected_backend.description.clone(),
                        },
                        properties,
                    });
                    output_connected.write(OutputConnected { name: record.output_name.clone() });
                }
            }
            "disconnected" => {
                for (entity, output) in &existing_outputs {
                    if output.name == record.output_name {
                        commands.entity(entity).despawn();
                    }
                }
                output_disconnected.write(OutputDisconnected { name: record.output_name.clone() });
                known_outputs.remove(&record.output_name);
            }
            _ => {}
        }
    }

    output_registry.connected_outputs = known_outputs.into_iter().collect();
}

fn apply_output_server_requests_system(
    mut commands: Commands,
    selected_backend: Res<SelectedBackend>,
    mut output_registry: ResMut<BackendOutputRegistry>,
    mut pending_output_requests: ResMut<PendingOutputServerRequests>,
    mut outputs: Query<(Entity, &OutputDevice, &mut OutputProperties)>,
    mut output_connected: MessageWriter<OutputConnected>,
    mut output_disconnected: MessageWriter<OutputDisconnected>,
) {
    let mut deferred = Vec::new();
    let mut known_outputs = outputs
        .iter_mut()
        .map(|(entity, existing, _)| (existing.name.clone(), entity))
        .collect::<BTreeMap<_, _>>();

    for request in pending_output_requests.items.drain(..) {
        match request.action {
            OutputServerAction::Enable { output } => {
                if known_outputs.contains_key(&output) {
                    continue;
                }

                commands.spawn(OutputBundle {
                    output: OutputDevice {
                        name: output.clone(),
                        kind: output_kind_for_backend(selected_backend.kind.clone()),
                        make: format!("{:?}", selected_backend.kind),
                        model: selected_backend.description.clone(),
                    },
                    properties: default_output_properties(selected_backend.kind.clone()),
                });
                known_outputs.insert(output.clone(), Entity::PLACEHOLDER);
                if !output_registry.connected_outputs.iter().any(|name| name == &output) {
                    output_registry.connected_outputs.push(output.clone());
                    output_registry.connected_outputs.sort();
                }
                output_connected.write(OutputConnected { name: output });
            }
            OutputServerAction::Disable { output } => {
                let Some(entity) = known_outputs.get(&output).copied() else {
                    continue;
                };

                if entity != Entity::PLACEHOLDER {
                    commands.entity(entity).despawn();
                }
                known_outputs.remove(&output);
                output_registry.connected_outputs.retain(|name| name != &output);
                output_disconnected.write(OutputDisconnected { name: output });
            }
            OutputServerAction::Configure { output, mode, scale } => {
                let Some(configured_mode) = parse_output_mode(&mode) else {
                    tracing::warn!(output, mode, "ignoring invalid output mode request");
                    continue;
                };

                let Some(entity) = known_outputs.get(&output).copied() else {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Configure { output, mode, scale },
                    });
                    continue;
                };

                if entity == Entity::PLACEHOLDER {
                    deferred.push(OutputServerRequest {
                        action: OutputServerAction::Configure { output, mode, scale },
                    });
                    continue;
                }

                for (existing_entity, existing, mut properties) in &mut outputs {
                    if existing_entity != entity || existing.name != output {
                        continue;
                    }

                    properties.width = configured_mode.width;
                    properties.height = configured_mode.height;
                    properties.refresh_millihz = configured_mode.refresh_millihz;
                    if let Some(scale) = scale {
                        properties.scale = scale.max(1);
                    }
                    break;
                }
            }
        }
    }

    pending_output_requests.items = deferred;
}

fn output_matches_config(
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

fn default_output_name(kind: BackendKind) -> Option<&'static str> {
    match kind {
        BackendKind::Drm => Some("eDP-1"),
        BackendKind::Winit => Some("Winit-1"),
        BackendKind::Virtual => Some("Virtual-1"),
        BackendKind::Auto => None,
    }
}

fn apply_output_presentation_events_system(
    outputs: Query<&OutputDevice>,
    mut pending_presentation_events: ResMut<PendingOutputPresentationEvents>,
    mut presentation_state: ResMut<OutputPresentationState>,
) {
    let known_outputs = outputs.iter().map(|output| output.name.clone()).collect::<BTreeSet<_>>();
    let mut timelines = presentation_state
        .outputs
        .drain(..)
        .filter(|timeline| known_outputs.contains(&timeline.output_name))
        .map(|timeline| (timeline.output_name.clone(), timeline))
        .collect::<BTreeMap<_, _>>();

    for event in pending_presentation_events.items.drain(..) {
        if !known_outputs.contains(&event.output_name) {
            continue;
        }

        timelines.insert(
            event.output_name.clone(),
            OutputPresentationTimeline {
                output_name: event.output_name,
                refresh_interval_nanos: event.refresh_interval_nanos,
                present_time_nanos: event.present_time_nanos,
                sequence: event.sequence,
            },
        );
    }

    presentation_state.outputs = timelines.into_values().collect();
}

fn default_output_properties(kind: BackendKind) -> OutputProperties {
    match kind {
        BackendKind::Drm => {
            OutputProperties { width: 2560, height: 1440, refresh_millihz: 144_000, scale: 1 }
        }
        BackendKind::Virtual => {
            OutputProperties { width: 1920, height: 1080, refresh_millihz: 60_000, scale: 1 }
        }
        BackendKind::Winit | BackendKind::Auto => {
            OutputProperties { width: 1280, height: 720, refresh_millihz: 60_000, scale: 1 }
        }
    }
}

fn output_kind_for_backend(kind: BackendKind) -> OutputKind {
    match kind {
        BackendKind::Drm => OutputKind::Physical,
        BackendKind::Winit => OutputKind::Nested,
        BackendKind::Virtual | BackendKind::Auto => OutputKind::Virtual,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ParsedOutputMode {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) refresh_millihz: u32,
}

pub(crate) fn parse_output_mode(mode: &str) -> Option<ParsedOutputMode> {
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

fn refresh_interval_nanos(refresh_millihz: u32) -> u64 {
    if refresh_millihz == 0 {
        return 0;
    }

    1_000_000_000_000_u64 / u64::from(refresh_millihz)
}

fn quantized_present_time_nanos(
    now_nanos: u64,
    anchor_nanos: u64,
    refresh_interval_nanos: u64,
) -> u64 {
    if refresh_interval_nanos == 0 {
        return now_nanos;
    }

    let elapsed_nanos = now_nanos.saturating_sub(anchor_nanos);
    let completed_intervals = elapsed_nanos / refresh_interval_nanos;
    anchor_nanos.saturating_add(completed_intervals.saturating_mul(refresh_interval_nanos))
}

fn duration_to_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

pub(crate) fn emit_present_completion_events(
    backend_kind: BackendKind,
    selected_backend: &SelectedBackend,
    outputs: &Query<(&OutputDevice, &OutputProperties)>,
    pending_presentation_events: &mut PendingOutputPresentationEvents,
    presentation_runtime: &mut OutputPresentationRuntime,
    monotonic_clock: &mut Option<Clock<Monotonic>>,
) {
    if selected_backend.kind != backend_kind {
        return;
    }

    let now = Duration::from(monotonic_clock.get_or_insert_with(Clock::<Monotonic>::new).now());
    emit_present_completion_events_at(
        backend_kind,
        selected_backend,
        outputs,
        pending_presentation_events,
        presentation_runtime,
        duration_to_nanos(now),
    );
}

pub(crate) fn emit_present_completion_events_at(
    backend_kind: BackendKind,
    selected_backend: &SelectedBackend,
    outputs: &Query<(&OutputDevice, &OutputProperties)>,
    pending_presentation_events: &mut PendingOutputPresentationEvents,
    presentation_runtime: &mut OutputPresentationRuntime,
    now_nanos: u64,
) {
    if selected_backend.kind != backend_kind {
        return;
    }

    let mut known_outputs = BTreeSet::new();

    for (output, properties) in outputs.iter() {
        known_outputs.insert(output.name.clone());
        let refresh_interval_nanos = refresh_interval_nanos(properties.refresh_millihz);
        let clock = presentation_runtime.outputs.entry(output.name.clone()).or_insert(
            OutputPresentationClock {
                anchor_nanos: now_nanos,
                refresh_interval_nanos,
                sequence: 0,
            },
        );

        if clock.refresh_interval_nanos != refresh_interval_nanos {
            clock.anchor_nanos = now_nanos;
            clock.refresh_interval_nanos = refresh_interval_nanos;
            clock.sequence = 0;
        }

        let present_time_nanos =
            quantized_present_time_nanos(now_nanos, clock.anchor_nanos, refresh_interval_nanos);

        let sequence = if refresh_interval_nanos == 0 {
            clock.sequence = clock.sequence.saturating_add(1);
            clock.sequence
        } else {
            let elapsed = present_time_nanos.saturating_sub(clock.anchor_nanos);
            clock.sequence = (elapsed / refresh_interval_nanos).saturating_add(1);
            clock.sequence
        };

        pending_presentation_events.items.push(OutputPresentationEventRecord {
            output_name: output.name.clone(),
            refresh_interval_nanos,
            present_time_nanos,
            sequence,
        });
    }

    presentation_runtime.outputs.retain(|output_name, _| known_outputs.contains(output_name));
}
