use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bevy_ecs::prelude::{Query, ResMut};
use nekoland_ecs::components::{OutputDevice, OutputProperties};
use nekoland_ecs::resources::{
    OutputPresentationEventRecord, OutputPresentationState, OutputPresentationTimeline,
    PendingOutputPresentationEvents,
};
use smithay::utils::{Clock, Monotonic};

/// Maintains a per-output present-timeline cursor so backend-specific completion timestamps can
/// be projected into the normalized ECS presentation stream.
#[derive(Debug, Default)]
pub struct OutputPresentationRuntime {
    outputs: BTreeMap<String, OutputPresentationClock>,
}

#[derive(Debug, Clone, Copy)]
struct OutputPresentationClock {
    /// Time anchor from which quantized presentation timestamps are derived.
    anchor_nanos: u64,
    /// Refresh interval currently associated with the output timeline.
    refresh_interval_nanos: u64,
    /// Last sequence number emitted for the output.
    sequence: u64,
}

/// Apply backend-produced presentation events into the normalized ECS snapshot.
pub fn apply_output_presentation_events_system(
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

    for event in pending_presentation_events.drain() {
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

/// Emit completion events using the current monotonic clock as the backend time source.
pub fn emit_present_completion_events(
    outputs: impl IntoIterator<Item = (String, OutputProperties)>,
    pending_presentation_events: &mut PendingOutputPresentationEvents,
    presentation_runtime: &mut OutputPresentationRuntime,
    monotonic_clock: &mut Option<Clock<Monotonic>>,
) {
    let now = Duration::from(monotonic_clock.get_or_insert_with(Clock::<Monotonic>::new).now());
    emit_present_completion_events_at(
        outputs,
        pending_presentation_events,
        presentation_runtime,
        duration_to_nanos(now),
    );
}

/// Emit completion events at one explicit timestamp.
///
/// Backends use this helper after they decide which outputs should be considered
/// presented for the current frame; this function only normalizes timestamps and
/// sequence numbers.
pub fn emit_present_completion_events_at(
    outputs: impl IntoIterator<Item = (String, OutputProperties)>,
    pending_presentation_events: &mut PendingOutputPresentationEvents,
    presentation_runtime: &mut OutputPresentationRuntime,
    now_nanos: u64,
) {
    let mut known_outputs = BTreeSet::new();

    for (output_name, properties) in outputs {
        known_outputs.insert(output_name.clone());
        let refresh_interval_nanos = refresh_interval_nanos(properties.refresh_millihz);
        let clock = presentation_runtime.outputs.entry(output_name.clone()).or_insert(
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

        pending_presentation_events.push(OutputPresentationEventRecord {
            output_name,
            refresh_interval_nanos,
            present_time_nanos,
            sequence,
        });
    }

    presentation_runtime.outputs.retain(|output_name, _| known_outputs.contains(output_name));
}

/// Convert millihertz refresh values into one-frame intervals in nanoseconds.
fn refresh_interval_nanos(refresh_millihz: u32) -> u64 {
    if refresh_millihz == 0 {
        return 0;
    }

    1_000_000_000_000_u64 / u64::from(refresh_millihz)
}

/// Snap one timestamp down to the closest completed refresh interval.
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

/// Lossily convert a `Duration` into a `u64` nanosecond count.
fn duration_to_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}
