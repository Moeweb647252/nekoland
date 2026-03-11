use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::ResMut;
use nekoland_ecs::events::KeyPress;
use nekoland_ecs::resources::{
    BackendInputAction, ModifierState, PendingBackendInputEvents, PendingInputEvents,
};

pub fn keyboard_input_system(
    mut key_events: MessageWriter<KeyPress>,
    mut modifiers: ResMut<ModifierState>,
    mut pending_backend_input_events: ResMut<PendingBackendInputEvents>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    let mut deferred = Vec::new();

    for event in pending_backend_input_events.items.drain(..) {
        match event.action {
            BackendInputAction::Key { keycode, pressed } => {
                update_modifier_state(&mut modifiers, keycode, pressed);
                key_events.write(KeyPress { keycode, pressed });
                pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
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
                pending_input_events.items.push(nekoland_ecs::resources::InputEventRecord {
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

    pending_backend_input_events.items = deferred;
}

fn update_modifier_state(modifiers: &mut ModifierState, keycode: u32, pressed: bool) {
    match keycode {
        37 | 105 => modifiers.ctrl = pressed,
        50 | 62 => modifiers.shift = pressed,
        64 | 108 => modifiers.alt = pressed,
        133 | 134 => modifiers.logo = pressed,
        _ => {}
    }
}
