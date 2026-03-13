use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::ResMut;
use nekoland_ecs::events::{PointerButton, PointerMotion};
use nekoland_ecs::resources::{
    BackendInputAction, GlobalPointerPosition, PendingBackendInputEvents, PendingInputEvents,
};

/// Consumes pointer-related backend input records, updates the shared pointer position, and emits
/// higher-level ECS pointer messages plus human-readable input log entries.
pub fn pointer_input_system(
    mut pointer: ResMut<GlobalPointerPosition>,
    mut button_events: MessageWriter<PointerButton>,
    mut motion_events: MessageWriter<PointerMotion>,
    mut pending_backend_input_events: ResMut<PendingBackendInputEvents>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    // Leave keyboard/touch events in the backend queue so their dedicated systems can handle them
    // later in the same input phase.
    let mut deferred = Vec::new();

    for event in pending_backend_input_events.drain() {
        match event.action {
            BackendInputAction::PointerMoved { x, y } => {
                pointer.x = x;
                pointer.y = y;
                motion_events.write(PointerMotion { x, y });
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("moved to ({x:.1}, {y:.1})"),
                });
            }
            BackendInputAction::PointerButton { button_code, pressed } => {
                button_events.write(PointerButton { button_code, pressed });
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!(
                        "button {button_code} {}",
                        if pressed { "pressed" } else { "released" }
                    ),
                });
            }
            BackendInputAction::PointerAxis { horizontal, vertical } => {
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("axis ({horizontal:.1}, {vertical:.1})"),
                });
            }
            _ => deferred.push(event),
        }
    }

    pending_backend_input_events.replace(deferred);
}
