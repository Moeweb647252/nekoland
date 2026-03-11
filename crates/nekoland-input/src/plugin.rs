use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::InputSchedule;
use nekoland_ecs::events::{
    ExternalCommandFailed, ExternalCommandLaunched, GestureSwipe, KeyPress, PointerMotion,
};
use nekoland_ecs::resources::{
    CommandHistoryState, GlobalPointerPosition, KeyboardFocusState, ModifierState,
    PendingBackendInputEvents, PendingExternalCommandRequests, PendingInputEvents,
    PendingOutputServerRequests, PendingWindowServerRequests, PendingWorkspaceServerRequests,
};

use crate::{commands, gestures, keybindings, keyboard, pointer, seat_manager, touch};

#[derive(Debug, Default, Clone, Copy)]
pub struct InputPlugin;

impl NekolandPlugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlobalPointerPosition>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<ModifierState>()
            .init_resource::<CommandHistoryState>()
            .init_resource::<PendingBackendInputEvents>()
            .init_resource::<PendingExternalCommandRequests>()
            .init_resource::<PendingInputEvents>()
            .init_resource::<PendingOutputServerRequests>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingWorkspaceServerRequests>()
            .init_resource::<keybindings::KeybindingEngine>()
            .init_resource::<commands::StartupCommandState>()
            .init_resource::<seat_manager::SeatManager>()
            .add_message::<KeyPress>()
            .add_message::<PointerMotion>()
            .add_message::<GestureSwipe>()
            .add_message::<ExternalCommandLaunched>()
            .add_message::<ExternalCommandFailed>()
            .add_systems(
                InputSchedule,
                (
                    keyboard::keyboard_input_system,
                    pointer::pointer_input_system,
                    touch::touch_input_system,
                    gestures::gesture_recognition_system,
                    keybindings::keybinding_dispatch_system,
                    commands::startup_command_queue_system,
                    commands::external_command_launch_system,
                    commands::command_history_system,
                    seat_manager::seat_management_system,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::message::MessageReader;
    use bevy_ecs::prelude::{ResMut, Resource};
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::InputSchedule;
    use nekoland_ecs::events::{KeyPress, PointerMotion};
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CompositorClock, CompositorConfig,
        GlobalPointerPosition, ModifierState, PendingBackendInputEvents, PendingInputEvents,
    };

    use super::InputPlugin;
    use crate::seat_manager;

    #[derive(Debug, Default, Resource)]
    struct InputAudit {
        key_events: Vec<(u32, bool)>,
        pointer_events: Vec<(f64, f64)>,
    }

    #[test]
    fn backend_input_events_update_input_state_and_messages() {
        let mut app = NekolandApp::new("input-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(CompositorConfig::default())
            .add_plugin(InputPlugin);

        app.inner_mut().init_resource::<InputAudit>().add_systems(
            InputSchedule,
            capture_input_messages.after(seat_manager::seat_management_system),
        );
        app.inner_mut().insert_resource(PendingBackendInputEvents {
            items: vec![
                BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::Key { keycode: 133, pressed: true },
                },
                BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::Key { keycode: 36, pressed: true },
                },
                BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::PointerMoved { x: 320.5, y: 128.0 },
                },
            ],
        });

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let audit = world.get_resource::<InputAudit>().expect("input audit should be initialized");
        let pointer = world
            .get_resource::<GlobalPointerPosition>()
            .expect("pointer position should be initialized");
        let modifiers =
            world.get_resource::<ModifierState>().expect("modifier state should be initialized");
        let pending_input_events = world
            .get_resource::<PendingInputEvents>()
            .expect("pending input events should be initialized");
        let pending_backend_input_events = world
            .get_resource::<PendingBackendInputEvents>()
            .expect("backend input queue should be initialized");

        assert_eq!(audit.key_events, vec![(133, true), (36, true)]);
        assert_eq!(audit.pointer_events, vec![(320.5, 128.0)]);
        assert_eq!((pointer.x, pointer.y), (320.5, 128.0));
        assert!(modifiers.logo, "logo modifier should track the backend key press");
        assert!(
            pending_input_events
                .items
                .iter()
                .any(|event| event.detail.contains("keycode 133 pressed"))
        );
        assert!(
            pending_input_events
                .items
                .iter()
                .any(|event| event.detail.contains("moved to (320.5, 128.0)"))
        );
        assert!(
            pending_backend_input_events.items.is_empty(),
            "backend input queue should be fully drained by the input systems"
        );
    }

    fn capture_input_messages(
        mut key_events: MessageReader<KeyPress>,
        mut pointer_events: MessageReader<PointerMotion>,
        mut audit: ResMut<InputAudit>,
    ) {
        for event in key_events.read() {
            audit.key_events.push((event.keycode, event.pressed));
        }

        for event in pointer_events.read() {
            audit.pointer_events.push((event.x, event.y));
        }
    }
}
