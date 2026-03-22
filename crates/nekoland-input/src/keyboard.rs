use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::events::KeyPress;
use nekoland_ecs::resources::{
    InputEventRecord, ModifierState, PendingInputEvents, PlatformInputAction, PressedKeys,
    WaylandIngress, update_modifier_state,
};

/// Consumes only keyboard-related backend input records, updates coarse modifier state, and
/// forwards key presses into both ECS messages and the human-readable input event log.
pub fn keyboard_input_system(
    wayland_ingress: Option<Res<'_, WaylandIngress>>,
    mut key_events: MessageWriter<KeyPress>,
    mut pressed_keys: ResMut<PressedKeys>,
    mut modifiers: ResMut<ModifierState>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    pressed_keys.clear_frame_transitions();

    let Some(wayland_ingress) = wayland_ingress else {
        return;
    };

    for event in wayland_ingress.platform_input_events.iter() {
        match &event.action {
            PlatformInputAction::Key { keycode, pressed } => {
                pressed_keys.record_key(*keycode, *pressed);
                update_modifier_state(&mut modifiers, *keycode, *pressed);
                key_events.write(KeyPress { keycode: *keycode, pressed: *pressed });
                pending_input_events.push(InputEventRecord {
                    source: format!("keyboard:{}", event.device),
                    detail: format!(
                        "keycode {keycode} {}",
                        if *pressed { "pressed" } else { "released" }
                    ),
                });
            }
            PlatformInputAction::FocusChanged { focused } => {
                if !focused {
                    pressed_keys.reset_all();
                    *modifiers = ModifierState::default();
                }
                pending_input_events.push(InputEventRecord {
                    source: format!("keyboard:{}", event.device),
                    detail: if *focused {
                        "focus gained".to_owned()
                    } else {
                        "focus lost".to_owned()
                    },
                });
            }
            _ => {}
        }
    }
}
