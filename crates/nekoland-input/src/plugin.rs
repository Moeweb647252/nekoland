use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::InputSchedule;
use nekoland_ecs::events::{GestureSwipe, KeyPress, PointerButton, PointerMotion};
use nekoland_ecs::resources::{
    FocusedOutputState, GlobalPointerPosition, KeyboardFocusState, ModifierState,
    PendingExternalCommandRequests, PendingInputEvents, PendingOutputControls,
    PendingWindowControls, PendingWorkspaceControls, PhysicalPointerPosition, PointerDelta,
    PressedKeys, SeatRegistry, ViewportPointerPanState, WaylandIngress,
};

use crate::{gestures, keybindings, keyboard, pointer, seat_manager, touch};

#[derive(Debug, Default, Clone, Copy)]
pub struct InputPlugin;

impl NekolandPlugin for InputPlugin {
    /// Register input resources plus the ordered input-decoding pipeline.
    fn build(&self, app: &mut App) {
        app.init_resource::<GlobalPointerPosition>()
            .init_resource::<PhysicalPointerPosition>()
            .init_resource::<PointerDelta>()
            .init_resource::<ViewportPointerPanState>()
            .init_resource::<FocusedOutputState>()
            .init_resource::<WaylandIngress>()
            .init_resource::<KeyboardFocusState>()
            .init_resource::<ModifierState>()
            .init_resource::<PressedKeys>()
            .init_resource::<PendingExternalCommandRequests>()
            .init_resource::<PendingInputEvents>()
            .init_resource::<PendingOutputControls>()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWorkspaceControls>()
            .init_resource::<keybindings::CompiledKeybindings>()
            .init_resource::<SeatRegistry>()
            .add_message::<KeyPress>()
            .add_message::<PointerButton>()
            .add_message::<PointerMotion>()
            .add_message::<GestureSwipe>()
            .add_systems(
                InputSchedule,
                // Keep backend event decoding ahead of higher-level gesture/keybinding logic so
                // later systems see the current modifier state and pointer position.
                (
                    keyboard::keyboard_input_system,
                    pointer::pointer_input_system,
                    keybindings::reload_keybindings_system,
                    pointer::viewport_pointer_pan_system,
                    pointer::cursor_motion_system,
                    pointer::focused_output_tracking_system,
                    touch::touch_input_system,
                    gestures::gesture_recognition_system,
                    keybindings::window_keybinding_system,
                    keybindings::workspace_keybinding_system,
                    keybindings::output_keybinding_system,
                    keybindings::command_keybinding_system,
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
    use nekoland_config::resources::CompositorConfig;
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::InputSchedule;
    use nekoland_ecs::events::{KeyPress, PointerMotion};
    use nekoland_ecs::resources::PendingInputEvents;
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CompositorClock, FocusedOutputState,
        GlobalPointerPosition, ModifierState, OutputGeometrySnapshot, OutputSnapshotState,
        PressedKeys, WaylandIngress,
    };

    use super::InputPlugin;
    use crate::seat_manager;

    #[derive(Debug, Default, Resource)]
    struct InputAudit {
        /// Key events observed from the message bus during the test.
        key_events: Vec<(u32, bool)>,
        /// Pointer-motion events observed from the message bus during the test.
        pointer_events: Vec<(f64, f64)>,
    }

    #[test]
    fn backend_input_events_update_input_state_and_messages() {
        let mut app = NekolandApp::new("input-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(CompositorConfig::default())
            .insert_resource(WaylandIngress {
                platform_input_events:
                    nekoland_ecs::resources::PendingPlatformInputEvents::from_items(vec![
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
                    ]),
                ..WaylandIngress::default()
            })
            .add_plugin(InputPlugin);

        app.inner_mut().init_resource::<InputAudit>().add_systems(
            InputSchedule,
            capture_input_messages.after(seat_manager::seat_management_system),
        );

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        let world = app.inner().world();
        let Some(audit) = world.get_resource::<InputAudit>() else {
            panic!("input audit should be initialized");
        };
        let Some(pointer) = world.get_resource::<GlobalPointerPosition>() else {
            panic!("pointer position should be initialized");
        };
        let Some(modifiers) = world.get_resource::<ModifierState>() else {
            panic!("modifier state should be initialized");
        };
        let Some(pressed_keys) = world.get_resource::<PressedKeys>() else {
            panic!("pressed keys should be initialized");
        };
        let Some(pending_input_events) = world.get_resource::<PendingInputEvents>() else {
            panic!("pending input events should be initialized");
        };

        assert_eq!(audit.key_events, vec![(133, true), (36, true)]);
        assert_eq!(audit.pointer_events, vec![(320.5, 128.0)]);
        assert_eq!((pointer.x, pointer.y), (320.5, 128.0));
        assert!(modifiers.logo, "logo modifier should track the backend key press");
        assert!(pressed_keys.held().contains(&133), "pressed key set should track held keys");
        assert!(
            pending_input_events.iter().any(|event| event.detail.contains("keycode 133 pressed"))
        );
        assert!(
            pending_input_events
                .iter()
                .any(|event| event.detail.contains("moved to (320.5, 128.0)"))
        );
    }

    #[test]
    fn focused_output_tracking_uses_wayland_ingress_output_snapshots() {
        let mut app = NekolandApp::new("input-output-snapshot-sync-test");
        app.insert_resource(CompositorClock::default())
            .insert_resource(CompositorConfig::default())
            .insert_resource(WaylandIngress {
                output_snapshots: OutputSnapshotState {
                    outputs: vec![OutputGeometrySnapshot {
                        output_id: nekoland_ecs::components::OutputId(7),
                        name: "DP-1".to_owned(),
                        x: 10,
                        y: 20,
                        width: 1920,
                        height: 1080,
                        scale: 2,
                        refresh_millihz: 60_000,
                    }],
                },
                ..WaylandIngress::default()
            })
            .insert_resource(GlobalPointerPosition { x: 500.0, y: 500.0 })
            .add_plugin(InputPlugin);

        app.inner_mut().world_mut().run_schedule(InputSchedule);

        assert_eq!(
            app.inner().world().resource::<FocusedOutputState>().id,
            Some(nekoland_ecs::components::OutputId(7))
        );
    }

    /// Capture input messages after the plugin drained backend input events.
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
