use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Local, Res};
use nekoland_ecs::events::GestureSwipe;
use nekoland_ecs::resources::GlobalPointerPosition;

/// Temporary gesture recognizer that derives coarse swipe events from pointer movement buckets.
///
/// This is a placeholder for real gesture input and currently only emits a synthetic
/// left-to-right three-finger swipe signal.
pub fn gesture_recognition_system(
    pointer: Res<GlobalPointerPosition>,
    mut gesture_events: MessageWriter<GestureSwipe>,
    mut last_swipe_bucket: Local<i32>,
) {
    let bucket = (pointer.x / 32.0) as i32;
    if bucket > *last_swipe_bucket {
        gesture_events.write(GestureSwipe { delta_x: 1.0, delta_y: 0.0, fingers: 3 });
        *last_swipe_bucket = bucket;
    }
}
