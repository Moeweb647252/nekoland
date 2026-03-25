//! Frame-local animation sampling used by render-side appearance and projection snapshots.
//!
//! This module is primarily a render-internal data model. Type- and function-level documentation
//! captures the intent, while individual enum variants and fields intentionally rely on their
//! containing type docs to avoid repeating obvious shape information.
#![allow(missing_docs)]

use std::collections::BTreeMap;

use bevy_ecs::prelude::{Res, ResMut, Resource};
use nekoland_ecs::resources::{CompositorClock, RenderRect};

use crate::scene_source::{RenderInstanceKey, RenderSourceKey};

/// Stable animation binding target resolved on the render side.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimationBindingKey {
    Source(RenderSourceKey),
    Instance(RenderInstanceKey),
}

/// One animatable property in the render/runtime visual layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimationProperty {
    Opacity,
    Rect,
    ClipRect,
}

/// One sampled animation value.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationValue {
    Float(f32),
    Rect(RenderRect),
}

/// Supported easing functions for timeline sampling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnimationEasing {
    #[default]
    Linear,
    EaseInOut,
}

/// One active track for a binding/property pair.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationTrack {
    pub property: AnimationProperty,
    pub from: AnimationValue,
    pub to: AnimationValue,
    pub start_uptime_millis: u128,
    pub duration_millis: u32,
    pub easing: AnimationEasing,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct AnimationTrackKey {
    binding: AnimationBindingKey,
    property: AnimationProperty,
}

/// Frame-local animation samples derived from the active timeline store.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct AnimationTimelineStore {
    tracks: BTreeMap<AnimationTrackKey, AnimationTrack>,
    sampled_values: BTreeMap<AnimationBindingKey, BTreeMap<AnimationProperty, AnimationValue>>,
    last_tick_uptime_millis: Option<u128>,
}

impl AnimationTimelineStore {
    /// Inserts or replaces the track for one binding/property pair.
    pub fn upsert_track(&mut self, binding: AnimationBindingKey, track: AnimationTrack) {
        let key = AnimationTrackKey { binding, property: track.property };
        self.tracks.insert(key, track);
    }

    /// Removes one track and any sampled value previously derived from it.
    pub fn remove_track(&mut self, binding: &AnimationBindingKey, property: AnimationProperty) {
        self.tracks.remove(&AnimationTrackKey { binding: binding.clone(), property });
        if let Some(samples) = self.sampled_values.get_mut(binding) {
            samples.remove(&property);
            if samples.is_empty() {
                self.sampled_values.remove(binding);
            }
        }
    }

    /// Returns the last sampled value for the provided binding/property pair.
    pub fn sampled_value(
        &self,
        binding: &AnimationBindingKey,
        property: AnimationProperty,
    ) -> Option<&AnimationValue> {
        self.sampled_values.get(binding).and_then(|values| values.get(&property))
    }

    /// Retains only tracks accepted by the provided predicate and clears rejected samples.
    pub fn retain_tracks<F>(&mut self, mut retain: F)
    where
        F: FnMut(&AnimationBindingKey, AnimationProperty, &AnimationTrack) -> bool,
    {
        let mut removed = Vec::new();
        self.tracks.retain(|key, track| {
            let keep = retain(&key.binding, key.property, track);
            if !keep {
                removed.push((key.binding.clone(), key.property));
            }
            keep
        });

        for (binding, property) in removed {
            if let Some(values) = self.sampled_values.get_mut(&binding) {
                values.remove(&property);
                if values.is_empty() {
                    self.sampled_values.remove(&binding);
                }
            }
        }
    }

    /// Returns the elapsed time since the previous animation tick in milliseconds.
    pub fn delta_millis(&self, current_uptime_millis: u128) -> u32 {
        self.last_tick_uptime_millis
            .map(|previous| {
                current_uptime_millis.saturating_sub(previous).min(u128::from(u32::MAX))
            })
            .unwrap_or_else(|| current_uptime_millis.min(u128::from(u32::MAX))) as u32
    }
}

/// Samples every active animation track against the compositor clock.
pub fn advance_animation_timelines_system(
    clock: Option<Res<'_, CompositorClock>>,
    mut timelines: ResMut<'_, AnimationTimelineStore>,
) {
    let current_uptime_millis =
        clock.as_deref().map(|clock| clock.uptime_millis).unwrap_or_default();
    let mut next_samples =
        BTreeMap::<AnimationBindingKey, BTreeMap<AnimationProperty, AnimationValue>>::new();

    for (key, track) in &timelines.tracks {
        if let Some(sample) = sample_track(track, current_uptime_millis) {
            next_samples.entry(key.binding.clone()).or_default().insert(track.property, sample);
        }
    }

    timelines.sampled_values = next_samples;
    timelines.last_tick_uptime_millis = Some(current_uptime_millis);
}

fn sample_track(track: &AnimationTrack, current_uptime_millis: u128) -> Option<AnimationValue> {
    let duration = u128::from(track.duration_millis.max(1));
    let elapsed = current_uptime_millis.saturating_sub(track.start_uptime_millis).min(duration);
    let progress = (elapsed as f32 / duration as f32).clamp(0.0, 1.0);
    let eased = ease_progress(progress, track.easing);

    match (&track.from, &track.to) {
        (AnimationValue::Float(from), AnimationValue::Float(to)) => {
            Some(AnimationValue::Float(interpolate_float(*from, *to, eased)))
        }
        (AnimationValue::Rect(from), AnimationValue::Rect(to)) => {
            Some(AnimationValue::Rect(interpolate_rect(*from, *to, eased)))
        }
        _ => None,
    }
}

fn ease_progress(progress: f32, easing: AnimationEasing) -> f32 {
    match easing {
        AnimationEasing::Linear => progress,
        AnimationEasing::EaseInOut => {
            if progress <= 0.0 {
                0.0
            } else if progress >= 1.0 {
                1.0
            } else {
                progress * progress * (3.0 - 2.0 * progress)
            }
        }
    }
}

fn interpolate_float(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

fn interpolate_rect(from: RenderRect, to: RenderRect, t: f32) -> RenderRect {
    RenderRect {
        x: interpolate_float(from.x as f32, to.x as f32, t).round() as i32,
        y: interpolate_float(from.y as f32, to.y as f32, t).round() as i32,
        width: interpolate_float(from.width as f32, to.width as f32, t).round().max(0.0) as u32,
        height: interpolate_float(from.height as f32, to.height as f32, t).round().max(0.0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::system::System;
    use nekoland_ecs::resources::{CompositorClock, RenderRect};

    use crate::scene_source::{RenderInstanceKey, RenderSourceKey};

    use super::{
        AnimationBindingKey, AnimationEasing, AnimationProperty, AnimationTimelineStore,
        AnimationTrack, AnimationValue, advance_animation_timelines_system,
    };

    #[test]
    fn timelines_sample_source_bound_opacity() {
        let mut world = bevy_ecs::world::World::default();
        world.insert_resource(CompositorClock { frame: 1, uptime_millis: 50 });
        world.insert_resource(AnimationTimelineStore::default());
        world.resource_mut::<AnimationTimelineStore>().upsert_track(
            AnimationBindingKey::Source(RenderSourceKey::surface(11)),
            AnimationTrack {
                property: AnimationProperty::Opacity,
                from: AnimationValue::Float(0.0),
                to: AnimationValue::Float(1.0),
                start_uptime_millis: 0,
                duration_millis: 100,
                easing: AnimationEasing::Linear,
            },
        );

        let mut system =
            bevy_ecs::system::IntoSystem::into_system(advance_animation_timelines_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        assert_eq!(
            world.resource::<AnimationTimelineStore>().sampled_value(
                &AnimationBindingKey::Source(RenderSourceKey::surface(11)),
                AnimationProperty::Opacity,
            ),
            Some(&AnimationValue::Float(0.5))
        );
    }

    #[test]
    fn timelines_sample_instance_bound_rects() {
        let mut world = bevy_ecs::world::World::default();
        world.insert_resource(CompositorClock { frame: 1, uptime_millis: 100 });
        world.insert_resource(AnimationTimelineStore::default());
        let binding = AnimationBindingKey::Instance(RenderInstanceKey::new(
            RenderSourceKey::surface(22),
            nekoland_ecs::components::OutputId(3),
            0,
        ));
        world.resource_mut::<AnimationTimelineStore>().upsert_track(
            binding.clone(),
            AnimationTrack {
                property: AnimationProperty::Rect,
                from: AnimationValue::Rect(RenderRect { x: 0, y: 0, width: 100, height: 100 }),
                to: AnimationValue::Rect(RenderRect { x: 20, y: 10, width: 80, height: 90 }),
                start_uptime_millis: 0,
                duration_millis: 100,
                easing: AnimationEasing::EaseInOut,
            },
        );

        let mut system =
            bevy_ecs::system::IntoSystem::into_system(advance_animation_timelines_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        assert_eq!(
            world
                .resource::<AnimationTimelineStore>()
                .sampled_value(&binding, AnimationProperty::Rect,),
            Some(&AnimationValue::Rect(RenderRect { x: 20, y: 10, width: 80, height: 90 }))
        );
    }
}
