use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::ResMut;
use nekoland_ecs::events::KeyPress;
use nekoland_ecs::resources::{
    BackendInputAction, ModifierState, PendingBackendInputEvents, PendingInputEvents,
};

/// Consumes only keyboard-related backend input records, updates coarse modifier state, and
/// forwards key presses into both ECS messages and the human-readable input event log.
pub fn keyboard_input_system(
    mut key_events: MessageWriter<KeyPress>,
    mut modifiers: ResMut<ModifierState>,
    mut pending_backend_input_events: ResMut<PendingBackendInputEvents>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    // Leave non-keyboard backend events in the queue so pointer/touch systems can process them
    // later in the same frame.
    let mut deferred = Vec::new();

    for event in pending_backend_input_events.drain() {
        match event.action {
            BackendInputAction::Key { keycode, pressed } => {
                update_modifier_state(&mut modifiers, keycode, pressed);
                key_events.write(KeyPress { keycode, pressed });
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("keyboard:{}", event.device),
                    detail: format!(
                        "keycode {keycode} {}",
                        if pressed { "pressed" } else { "released" }
                    ),
                });
            }
            BackendInputAction::FocusChanged { focused } => {
                if !focused {
                    *modifiers = ModifierState::default();
                }
                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("keyboard:{}", event.device),
                    detail: if focused {
                        "focus gained".to_owned()
                    } else {
                        "focus lost".to_owned()
                    },
                });
            }
            _ => deferred.push(event),
        }
    }

    pending_backend_input_events.replace(deferred);
}

/// Maintains the simplified modifier snapshot used by keybinding logic.
///
/// These codes currently mirror the backend's XKB/X11-style keycode values, so the mapping is
/// intentionally kept local to avoid leaking raw numeric assumptions into other systems.
fn update_modifier_state(modifiers: &mut ModifierState, keycode: u32, pressed: bool) {
    match keycode {
        37 | 105 => modifiers.ctrl = pressed,
        50 | 62 => modifiers.shift = pressed,
        64 | 108 => modifiers.alt = pressed,
        133 | 134 => modifiers.logo = pressed,
        _ => {}
    }
}
