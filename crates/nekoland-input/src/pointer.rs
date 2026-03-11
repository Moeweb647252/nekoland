use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::ResMut;
use nekoland_ecs::events::PointerMotion;
use nekoland_ecs::resources::{
    BackendInputAction, GlobalPointerPosition, PendingBackendInputEvents, PendingInputEvents,
};

pub fn pointer_input_system(
    mut pointer: ResMut<GlobalPointerPosition>,
    mut motion_events: MessageWriter<PointerMotion>,
    mut pending_backend_input_events: ResMut<PendingBackendInputEvents>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    let mut deferred = Vec::new();

    for event in pending_backend_input_events.items.drain(..) {
        match event.action {
            BackendInputAction::PointerMoved { x, y } => {
                pointer.x = x;
                pointer.y = y;
                motion_events.write(PointerMotion { x, y });
                pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("moved to ({x:.1}, {y:.1})"),
                });
            }
            BackendInputAction::PointerButton { button_code, pressed } => {
                pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!(
                        "button {button_code} {}",
                        if pressed { "pressed" } else { "released" }
                    ),
                });
            }
            BackendInputAction::PointerAxis { horizontal, vertical } => {
                pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("axis ({horizontal:.1}, {vertical:.1})"),
                });
            }
            _ => deferred.push(event),
        }
    }

    pending_backend_input_events.items = deferred;
}
