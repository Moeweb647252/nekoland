use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Local, Query, Res, ResMut};
use nekoland_ecs::events::{PointerButton, PointerMotion};
use nekoland_ecs::resources::{
    BackendInputAction, CompositorConfig, FocusedOutputState, GlobalPointerPosition, KeyShortcut,
    PendingBackendInputEvents, PendingInputEvents, PendingOutputControls, PhysicalPointerPosition,
    PointerDelta, PressedKeys, ViewportPointerPanState,
};
use nekoland_ecs::selectors::OutputName;
use nekoland_ecs::views::OutputRuntime;

#[derive(Debug, Default)]
pub(crate) struct ViewportPointerPanGestureState {
    active_output: Option<OutputName>,
    remainder_x: f64,
    remainder_y: f64,
    engaged: bool,
}

/// Consumes pointer-related backend input records into raw pointer state, pointer deltas, and
/// higher-level button messages.
pub fn pointer_input_system(
    pointer: Res<GlobalPointerPosition>,
    mut physical_pointer: ResMut<PhysicalPointerPosition>,
    mut pointer_delta: ResMut<PointerDelta>,
    mut button_events: MessageWriter<PointerButton>,
    mut pending_backend_input_events: ResMut<PendingBackendInputEvents>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    pointer_delta.dx = 0.0;
    pointer_delta.dy = 0.0;

    let mut deferred = Vec::new();

    for event in pending_backend_input_events.drain() {
        match event.action {
            BackendInputAction::PointerMoved { x, y } => {
                if physical_pointer.needs_resync {
                    physical_pointer.x = x;
                    physical_pointer.y = y;
                    physical_pointer.initialized = true;
                    physical_pointer.needs_resync = false;

                    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                        source: format!("pointer:{}", event.device),
                        detail: format!("resynced to ({x:.1}, {y:.1})"),
                    });
                    continue;
                }

                let (previous_x, previous_y) = if physical_pointer.initialized {
                    (physical_pointer.x, physical_pointer.y)
                } else {
                    (pointer.x, pointer.y)
                };
                pointer_delta.dx += x - previous_x;
                pointer_delta.dy += y - previous_y;
                physical_pointer.x = x;
                physical_pointer.y = y;
                physical_pointer.initialized = true;
                physical_pointer.needs_resync = false;

                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("moved to ({x:.1}, {y:.1})"),
                });
            }
            BackendInputAction::PointerDelta { dx, dy } => {
                pointer_delta.dx += dx;
                pointer_delta.dy += dy;
                physical_pointer.initialized = false;
                physical_pointer.needs_resync = true;

                pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("delta ({dx:.1}, {dy:.1})"),
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
            BackendInputAction::FocusChanged { focused } => {
                if !focused {
                    physical_pointer.initialized = false;
                    physical_pointer.needs_resync = false;
                    pointer_delta.dx = 0.0;
                    pointer_delta.dy = 0.0;
                }
                deferred.push(event);
            }
            _ => deferred.push(event),
        }
    }

    pending_backend_input_events.replace(deferred);
}

/// Applies any unconsumed raw pointer delta to the logical cursor position shared by the rest of
/// the compositor.
pub fn cursor_motion_system(
    mut pointer: ResMut<GlobalPointerPosition>,
    mut pointer_delta: ResMut<PointerDelta>,
    mut motion_events: MessageWriter<PointerMotion>,
) {
    if pointer_delta.dx == 0.0 && pointer_delta.dy == 0.0 {
        return;
    }

    pointer.x += pointer_delta.dx;
    pointer.y += pointer_delta.dy;
    motion_events.write(PointerMotion { x: pointer.x, y: pointer.y });
    pointer_delta.dx = 0.0;
    pointer_delta.dy = 0.0;
}

pub fn focused_output_tracking_system(
    pointer: Res<GlobalPointerPosition>,
    mut focused_output: ResMut<FocusedOutputState>,
    outputs: Query<OutputRuntime>,
) {
    let next_output = outputs
        .iter()
        .find(|output| {
            let left = f64::from(output.placement.x);
            let top = f64::from(output.placement.y);
            let right = left + f64::from(output.properties.width.max(1));
            let bottom = top + f64::from(output.properties.height.max(1));
            pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom
        })
        .map(|output| output.name().to_owned())
        .or_else(|| {
            focused_output.name.as_ref().and_then(|current| {
                outputs.iter().any(|output| output.name() == current).then_some(current.clone())
            })
        });

    if focused_output.name != next_output {
        focused_output.name = next_output;
    }
}

/// Treats the configured viewport-pan shortcut plus pointer delta as direct viewport panning on
/// the focused output.
pub(crate) fn viewport_pointer_pan_system(
    config: Res<CompositorConfig>,
    pressed_keys: Res<PressedKeys>,
    focused_output: Res<FocusedOutputState>,
    mut pointer_delta: ResMut<PointerDelta>,
    mut viewport_pan: ResMut<ViewportPointerPanState>,
    mut pending_output_controls: ResMut<PendingOutputControls>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    mut gesture: Local<ViewportPointerPanGestureState>,
) {
    let combo_active =
        pressed_keys.is_pressed(&KeyShortcut::modifier_only(config.viewport_pan_modifiers));

    if !combo_active {
        gesture.active_output = None;
        gesture.remainder_x = 0.0;
        gesture.remainder_y = 0.0;
        gesture.engaged = false;
        viewport_pan.active = false;
        return;
    }

    if gesture.active_output.is_none() {
        gesture.active_output =
            focused_output.name.as_ref().map(|name| OutputName::from(name.clone()));
    }

    let Some(output_name) = gesture.active_output.clone() else {
        viewport_pan.active = false;
        return;
    };

    let delta_x = pointer_delta.dx + gesture.remainder_x;
    let delta_y = pointer_delta.dy + gesture.remainder_y;
    let pan_x = delta_x.trunc() as isize;
    let pan_y = delta_y.trunc() as isize;

    gesture.remainder_x = delta_x - pan_x as f64;
    gesture.remainder_y = delta_y - pan_y as f64;
    pointer_delta.dx = 0.0;
    pointer_delta.dy = 0.0;

    if pan_x == 0 && pan_y == 0 {
        viewport_pan.active = gesture.engaged;
        return;
    }

    gesture.engaged = true;
    pending_output_controls.named(output_name).pan_viewport_by(pan_x, pan_y);
    pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
        source: "pointer:viewport".to_owned(),
        detail: format!("panned viewport by ({pan_x}, {pan_y})"),
    });
    viewport_pan.active = true;
}

#[cfg(test)]
mod tests {
    use bevy_ecs::message::Messages;
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::RunSystemOnce;
    use nekoland_ecs::events::{PointerButton, PointerMotion};
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CompositorConfig, FocusedOutputState,
        GlobalPointerPosition, ModifierMask, PendingBackendInputEvents, PendingInputEvents,
        PendingOutputControls, PhysicalPointerPosition, PointerDelta, PressedKeys,
        ViewportPointerPanState,
    };
    use nekoland_ecs::selectors::{OutputName, OutputSelector};

    use super::{cursor_motion_system, pointer_input_system, viewport_pointer_pan_system};

    #[test]
    fn viewport_pointer_pan_consumes_pointer_delta_without_moving_cursor() {
        let mut world = World::default();
        world.insert_resource(CompositorConfig::default());
        world.insert_resource(GlobalPointerPosition { x: 20.0, y: 10.0 });
        world.insert_resource(PointerDelta { dx: 4.0, dy: 7.0 });
        world.insert_resource(PressedKeys::default());
        world.init_resource::<PendingOutputControls>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<ViewportPointerPanState>();
        world.init_resource::<Messages<PointerMotion>>();
        world.insert_resource(FocusedOutputState { name: Some("DP-1".to_owned()) });

        {
            let mut pressed_keys = world.resource_mut::<PressedKeys>();
            pressed_keys.record_key(133, true);
            pressed_keys.record_key(64, true);
        }

        world.run_system_once(viewport_pointer_pan_system).expect("viewport pan system should run");
        world.run_system_once(cursor_motion_system).expect("cursor motion system should run");

        let output_controls = world.resource::<PendingOutputControls>();
        let pointer = world.resource::<GlobalPointerPosition>();
        let viewport_pan = world.resource::<ViewportPointerPanState>();

        assert_eq!(
            output_controls.as_slice(),
            &[nekoland_ecs::resources::PendingOutputControl {
                selector: OutputSelector::Name(OutputName::from("DP-1")),
                enabled: None,
                configuration: None,
                viewport_origin: None,
                viewport_pan: Some(nekoland_ecs::resources::OutputViewportPan {
                    delta_x: 4,
                    delta_y: 7,
                }),
                center_viewport_on: None,
            }]
        );
        assert_eq!((pointer.x, pointer.y), (20.0, 10.0));
        assert!(viewport_pan.active);
    }

    #[test]
    fn viewport_pointer_pan_uses_configured_modifier_mask() {
        let mut world = World::default();
        let mut config = CompositorConfig::default();
        config.viewport_pan_modifiers = ModifierMask::new(true, false, true, false);
        world.insert_resource(config);
        world.insert_resource(PointerDelta { dx: 4.0, dy: 7.0 });
        world.insert_resource(PressedKeys::default());
        world.init_resource::<PendingOutputControls>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<ViewportPointerPanState>();
        world.insert_resource(FocusedOutputState { name: Some("DP-1".to_owned()) });

        {
            let mut pressed_keys = world.resource_mut::<PressedKeys>();
            pressed_keys.record_key(37, true);
            pressed_keys.record_key(50, true);
        }

        world.run_system_once(viewport_pointer_pan_system).expect("viewport pan system should run");

        let output_controls =
            world.get_resource::<PendingOutputControls>().expect("output controls should exist");
        let viewport_pan = world
            .get_resource::<ViewportPointerPanState>()
            .expect("viewport pan state should exist");

        assert_eq!(
            output_controls.as_slice(),
            &[nekoland_ecs::resources::PendingOutputControl {
                selector: OutputSelector::Name(OutputName::from("DP-1")),
                enabled: None,
                configuration: None,
                viewport_origin: None,
                viewport_pan: Some(nekoland_ecs::resources::OutputViewportPan {
                    delta_x: 4,
                    delta_y: 7,
                }),
                center_viewport_on: None,
            }]
        );
        assert!(viewport_pan.active, "configured modifiers should activate viewport pan");
    }

    #[test]
    fn cursor_motion_applies_unconsumed_pointer_delta() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 20.0, y: 10.0 });
        world.insert_resource(PointerDelta { dx: 4.0, dy: 7.0 });
        world.init_resource::<Messages<PointerMotion>>();

        world.run_system_once(cursor_motion_system).expect("cursor motion system should run");

        let pointer = world.resource::<GlobalPointerPosition>();
        let pointer_delta = world.resource::<PointerDelta>();
        assert_eq!((pointer.x, pointer.y), (24.0, 17.0));
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (0.0, 0.0));
    }

    #[test]
    fn pointer_input_resyncs_after_relative_motion_without_jumping() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 20.0, y: 10.0 });
        world.insert_resource(PhysicalPointerPosition::default());
        world.insert_resource(PointerDelta::default());
        world.init_resource::<PendingBackendInputEvents>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<Messages<PointerButton>>();
        world.init_resource::<Messages<PointerMotion>>();

        world.resource_mut::<PendingBackendInputEvents>().push(BackendInputEvent {
            device: "winit".to_owned(),
            action: BackendInputAction::PointerDelta { dx: 4.0, dy: 7.0 },
        });
        world.run_system_once(pointer_input_system).expect("pointer input system should run");
        world.run_system_once(cursor_motion_system).expect("cursor motion system should run");

        let pointer = world.resource::<GlobalPointerPosition>();
        let physical = world.resource::<PhysicalPointerPosition>();
        assert_eq!((pointer.x, pointer.y), (24.0, 17.0));
        assert!(!physical.initialized);
        assert!(physical.needs_resync);

        world.resource_mut::<PendingBackendInputEvents>().push(BackendInputEvent {
            device: "winit".to_owned(),
            action: BackendInputAction::PointerMoved { x: 300.0, y: 200.0 },
        });
        world.run_system_once(pointer_input_system).expect("pointer input system should run");

        let pointer_delta = world.resource::<PointerDelta>();
        let physical = world.resource::<PhysicalPointerPosition>();
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (0.0, 0.0));
        assert_eq!((physical.x, physical.y), (300.0, 200.0));
        assert!(physical.initialized);
        assert!(!physical.needs_resync);

        world.resource_mut::<PendingBackendInputEvents>().push(BackendInputEvent {
            device: "winit".to_owned(),
            action: BackendInputAction::PointerMoved { x: 302.0, y: 203.0 },
        });
        world.run_system_once(pointer_input_system).expect("pointer input system should run");

        let pointer_delta = world.resource::<PointerDelta>();
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (2.0, 3.0));
    }

    #[test]
    fn pointer_focus_loss_clears_physical_sample_and_pending_delta() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 20.0, y: 10.0 });
        world.insert_resource(PhysicalPointerPosition {
            x: 300.0,
            y: 200.0,
            initialized: true,
            needs_resync: true,
        });
        world.insert_resource(PointerDelta { dx: 9.0, dy: -4.0 });
        world.init_resource::<PendingBackendInputEvents>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<Messages<PointerButton>>();

        world.resource_mut::<PendingBackendInputEvents>().push(BackendInputEvent {
            device: "winit".to_owned(),
            action: BackendInputAction::FocusChanged { focused: false },
        });
        world.run_system_once(pointer_input_system).expect("pointer input system should run");

        let physical = world.resource::<PhysicalPointerPosition>();
        let pointer_delta = world.resource::<PointerDelta>();
        let pending_backend = world.resource::<PendingBackendInputEvents>();

        assert!(!physical.initialized);
        assert!(!physical.needs_resync);
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (0.0, 0.0));
        assert_eq!(
            pending_backend.as_slice(),
            &[BackendInputEvent {
                device: "winit".to_owned(),
                action: BackendInputAction::FocusChanged { focused: false },
            }]
        );
    }
}
