use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy_ecs::prelude::Resource;

use crate::components::OutputId;
use crate::resources::{CompositorClock, OutputPresentationState};

const FPS_SAMPLE_WINDOW_MILLIS: u128 = 500;
const FPS_SAMPLE_WINDOW_NANOS: u64 = 500_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LoopFpsSample {
    frame: u64,
    uptime_millis: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PresentFpsSample {
    sequence: u64,
    present_time_nanos: u64,
    observed_uptime_millis: u128,
}

/// Output-facing display state for one FPS metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpsHudMetricValue {
    /// The compositor does not have enough data yet to derive a metric.
    Unavailable,
    /// A rounded integer FPS value.
    Fps(u32),
}

/// Runtime-only FPS HUD state, including the temporary user override and sampling windows.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct FpsHudRuntimeState {
    pub override_enabled: Option<bool>,
    loop_samples: VecDeque<LoopFpsSample>,
    present_samples: BTreeMap<OutputId, VecDeque<PresentFpsSample>>,
    outputs_with_valid_present_fps: BTreeSet<OutputId>,
}

impl FpsHudRuntimeState {
    pub fn effective_enabled(&self, config_enabled: bool) -> bool {
        self.override_enabled.unwrap_or(config_enabled)
    }

    pub fn set_enabled_override(&mut self, enabled: bool) {
        self.override_enabled = Some(enabled);
    }

    pub fn toggle_enabled_override(&mut self, config_enabled: bool) -> bool {
        let next = !self.effective_enabled(config_enabled);
        self.override_enabled = Some(next);
        next
    }

    pub fn observe_loop_clock(&mut self, clock: &CompositorClock) {
        if self.loop_samples.back().is_some_and(|sample| sample.frame == clock.frame) {
            return;
        }

        self.loop_samples
            .push_back(LoopFpsSample { frame: clock.frame, uptime_millis: clock.uptime_millis });
        prune_loop_samples(&mut self.loop_samples, clock.uptime_millis);
    }

    pub fn observe_output_presentation(
        &mut self,
        presentation: &OutputPresentationState,
        uptime_millis: u128,
    ) {
        let known_outputs =
            presentation.outputs.iter().map(|timeline| timeline.output_id).collect::<BTreeSet<_>>();

        for timeline in &presentation.outputs {
            let samples = self.present_samples.entry(timeline.output_id).or_default();
            if samples.back().is_none_or(|sample| {
                sample.sequence != timeline.sequence
                    || sample.present_time_nanos != timeline.present_time_nanos
            }) {
                samples.push_back(PresentFpsSample {
                    sequence: timeline.sequence,
                    present_time_nanos: timeline.present_time_nanos,
                    observed_uptime_millis: uptime_millis,
                });
            }
            prune_present_samples(samples, timeline.present_time_nanos);
            if rounded_present_fps(samples).is_some() {
                self.outputs_with_valid_present_fps.insert(timeline.output_id);
            }
        }

        self.present_samples.retain(|output_id, _| known_outputs.contains(output_id));
        self.outputs_with_valid_present_fps.retain(|output_id| known_outputs.contains(output_id));
    }

    pub fn loop_fps(&self) -> FpsHudMetricValue {
        rounded_loop_fps(&self.loop_samples)
            .map_or(FpsHudMetricValue::Unavailable, FpsHudMetricValue::Fps)
    }

    pub fn present_fps(&self, output_id: OutputId, now_uptime_millis: u128) -> FpsHudMetricValue {
        let Some(samples) = self.present_samples.get(&output_id) else {
            return FpsHudMetricValue::Unavailable;
        };

        let last_observed = samples.back().map(|sample| sample.observed_uptime_millis);
        if self.outputs_with_valid_present_fps.contains(&output_id)
            && last_observed.is_some_and(|last| {
                now_uptime_millis.saturating_sub(last) >= FPS_SAMPLE_WINDOW_MILLIS
            })
        {
            return FpsHudMetricValue::Fps(0);
        }

        rounded_present_fps(samples).map_or(FpsHudMetricValue::Unavailable, FpsHudMetricValue::Fps)
    }
}

fn prune_loop_samples(samples: &mut VecDeque<LoopFpsSample>, now_uptime_millis: u128) {
    while samples.front().is_some_and(|sample| {
        now_uptime_millis.saturating_sub(sample.uptime_millis) > FPS_SAMPLE_WINDOW_MILLIS
    }) {
        samples.pop_front();
    }
}

fn prune_present_samples(
    samples: &mut VecDeque<PresentFpsSample>,
    current_present_time_nanos: u64,
) {
    while samples.front().is_some_and(|sample| {
        current_present_time_nanos.saturating_sub(sample.present_time_nanos)
            > FPS_SAMPLE_WINDOW_NANOS
    }) {
        samples.pop_front();
    }
}

fn rounded_loop_fps(samples: &VecDeque<LoopFpsSample>) -> Option<u32> {
    let first = samples.front()?;
    let last = samples.back()?;
    let elapsed_millis = last.uptime_millis.saturating_sub(first.uptime_millis);
    let frame_delta = last.frame.saturating_sub(first.frame);
    rounded_rate(frame_delta, elapsed_millis, 1_000.0)
}

fn rounded_present_fps(samples: &VecDeque<PresentFpsSample>) -> Option<u32> {
    let first = samples.front()?;
    let last = samples.back()?;
    let elapsed_nanos = last.present_time_nanos.saturating_sub(first.present_time_nanos);
    let sequence_delta = last.sequence.saturating_sub(first.sequence);
    rounded_rate(sequence_delta, u128::from(elapsed_nanos), 1_000_000_000.0)
}

fn rounded_rate(count_delta: u64, elapsed_units: u128, units_per_second: f64) -> Option<u32> {
    if count_delta == 0 || elapsed_units == 0 {
        return None;
    }

    Some(
        ((count_delta as f64 * units_per_second) / elapsed_units as f64)
            .round()
            .clamp(0.0, u32::MAX as f64) as u32,
    )
}

#[cfg(test)]
mod tests {
    use crate::components::OutputId;
    use crate::resources::{
        CompositorClock, FpsHudMetricValue, FpsHudRuntimeState, OutputPresentationState,
        OutputPresentationTimeline,
    };

    #[test]
    fn loop_fps_is_unavailable_until_two_samples_exist() {
        let mut state = FpsHudRuntimeState::default();
        state.observe_loop_clock(&CompositorClock { frame: 1, uptime_millis: 0 });
        assert_eq!(state.loop_fps(), FpsHudMetricValue::Unavailable);
    }

    #[test]
    fn loop_fps_uses_recent_clock_window() {
        let mut state = FpsHudRuntimeState::default();
        state.observe_loop_clock(&CompositorClock { frame: 1, uptime_millis: 0 });
        state.observe_loop_clock(&CompositorClock { frame: 31, uptime_millis: 500 });
        assert_eq!(state.loop_fps(), FpsHudMetricValue::Fps(60));
    }

    #[test]
    fn present_fps_uses_presentation_timeline_samples() {
        let mut state = FpsHudRuntimeState::default();
        let output_id = OutputId(7);
        state.observe_output_presentation(
            &OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id,
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 0,
                    sequence: 1,
                }],
            },
            0,
        );
        assert_eq!(state.present_fps(output_id, 0), FpsHudMetricValue::Unavailable);

        state.observe_output_presentation(
            &OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id,
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 500_000_000,
                    sequence: 31,
                }],
            },
            500,
        );
        assert_eq!(state.present_fps(output_id, 500), FpsHudMetricValue::Fps(60));
    }

    #[test]
    fn stale_present_fps_falls_back_to_zero_after_valid_samples() {
        let mut state = FpsHudRuntimeState::default();
        let output_id = OutputId(3);
        state.observe_output_presentation(
            &OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id,
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 0,
                    sequence: 1,
                }],
            },
            0,
        );
        state.observe_output_presentation(
            &OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id,
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 500_000_000,
                    sequence: 31,
                }],
            },
            500,
        );
        assert_eq!(state.present_fps(output_id, 500), FpsHudMetricValue::Fps(60));
        assert_eq!(state.present_fps(output_id, 1_100), FpsHudMetricValue::Fps(0));
    }

    #[test]
    fn toggle_override_uses_effective_state() {
        let mut state = FpsHudRuntimeState::default();
        assert!(state.toggle_enabled_override(false));
        assert_eq!(state.override_enabled, Some(true));
        assert!(state.effective_enabled(false));
        assert!(!state.toggle_enabled_override(true));
        assert_eq!(state.override_enabled, Some(false));
    }
}
