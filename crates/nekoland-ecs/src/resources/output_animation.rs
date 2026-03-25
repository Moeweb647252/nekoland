//! Authoritative output viewport animation state mirrored into shell policy.

#![allow(missing_docs)]

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::components::{OutputId, OutputViewport};

/// One active authoritative viewport animation for a single output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputViewportAnimation {
    pub from: OutputViewport,
    pub to: OutputViewport,
    pub start_uptime_millis: u128,
    pub duration_millis: u32,
}

impl OutputViewportAnimation {
    /// Samples the interpolated viewport at the given compositor uptime.
    pub fn sample(&self, current_uptime_millis: u128) -> OutputViewport {
        let duration = u128::from(self.duration_millis.max(1));
        let elapsed = current_uptime_millis.saturating_sub(self.start_uptime_millis).min(duration);
        let progress = (elapsed as f32 / duration as f32).clamp(0.0, 1.0);
        let eased = smoothstep(progress);

        OutputViewport {
            origin_x: interpolate_coord(self.from.origin_x, self.to.origin_x, eased),
            origin_y: interpolate_coord(self.from.origin_y, self.to.origin_y, eased),
        }
    }

    /// Returns whether the animation has reached its end time.
    pub fn is_complete(&self, current_uptime_millis: u128) -> bool {
        current_uptime_millis
            >= self.start_uptime_millis.saturating_add(u128::from(self.duration_millis.max(1)))
    }
}

/// Active authoritative viewport animations keyed by output.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputViewportAnimationState {
    outputs: BTreeMap<OutputId, OutputViewportAnimation>,
}

impl OutputViewportAnimationState {
    /// Returns the active animation for one output, if any.
    pub fn animation_for(&self, output_id: OutputId) -> Option<&OutputViewportAnimation> {
        self.outputs.get(&output_id)
    }

    /// Returns the current sampled viewport for one output.
    pub fn sampled_viewport(
        &self,
        output_id: OutputId,
        current: &OutputViewport,
        current_uptime_millis: u128,
    ) -> OutputViewport {
        self.animation_for(output_id)
            .map(|animation| animation.sample(current_uptime_millis))
            .unwrap_or_else(|| current.clone())
    }

    /// Starts or replaces the active animation for one output.
    pub fn start(&mut self, output_id: OutputId, animation: OutputViewportAnimation) {
        self.outputs.insert(output_id, animation);
    }

    /// Cancels any active animation for one output.
    pub fn cancel(&mut self, output_id: OutputId) {
        self.outputs.remove(&output_id);
    }

    /// Returns the ids of outputs that currently have active animations.
    pub fn output_ids(&self) -> Vec<OutputId> {
        self.outputs.keys().copied().collect()
    }
}

/// Tracks which outputs currently have an in-flight authoritative viewport animation.
#[derive(Resource, Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewportAnimationActivityState {
    pub active_outputs: BTreeSet<OutputId>,
}

impl ViewportAnimationActivityState {
    /// Returns whether the given output currently has an in-flight viewport animation.
    pub fn is_output_active(&self, output_id: OutputId) -> bool {
        self.active_outputs.contains(&output_id)
    }
}

fn smoothstep(progress: f32) -> f32 {
    if progress <= 0.0 {
        0.0
    } else if progress >= 1.0 {
        1.0
    } else {
        progress * progress * (3.0 - 2.0 * progress)
    }
}

fn interpolate_coord(from: isize, to: isize, t: f32) -> isize {
    (from as f32 + (to as f32 - from as f32) * t).round() as isize
}
