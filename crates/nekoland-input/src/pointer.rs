use bevy_ecs::message::{MessageReader, MessageWriter};
use bevy_ecs::prelude::{Local, Query, Res, ResMut};
use nekoland_ecs::events::{PointerButton, PointerMotion};
use nekoland_ecs::resources::{
    BackendInputAction, CompositorConfig, FocusedOutputState, GlobalPointerPosition, ModifierState,
    PendingBackendInputEvents, PendingInputEvents, PendingOutputControls,
    PendingProtocolInputEvents, ViewportPointerPanState,
};
use nekoland_ecs::selectors::OutputName;
use nekoland_ecs::views::OutputRuntime;

#[derive(Debug, Default)]
pub(crate) struct ViewportPointerPanGestureState {
    active_output: Option<OutputName>,
    last_pointer: Option<(f64, f64)>,
    remainder_x: f64,
    remainder_y: f64,
    engaged: bool,
}

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

/// Treats the configured viewport-pan modifier gesture plus pointer motion as direct viewport
/// panning on the focused output.
///
/// While the gesture is active, physical pointer motion is handled compositor-side instead of
/// being forwarded to protocol hover handling.
pub(crate) fn viewport_pointer_pan_system(
    config: Res<CompositorConfig>,
    modifiers: Res<ModifierState>,
    focused_output: Res<FocusedOutputState>,
    mut pointer_events: MessageReader<PointerMotion>,
    mut viewport_pan: ResMut<ViewportPointerPanState>,
    mut pending_output_controls: ResMut<PendingOutputControls>,
    mut pending_input_events: ResMut<PendingInputEvents>,
    pending_protocol_input_events: Option<ResMut<PendingProtocolInputEvents>>,
    mut gesture: Local<ViewportPointerPanGestureState>,
) {
    let combo_active = config.viewport_pan_modifiers.matches_required(&modifiers);

    if !combo_active {
        gesture.active_output = None;
        gesture.remainder_x = 0.0;
        gesture.remainder_y = 0.0;
        gesture.engaged = false;
    }

    if combo_active {
        if gesture.active_output.is_none() {
            gesture.active_output =
                focused_output.name.as_ref().map(|name| OutputName::from(name.clone()));
        }
        if let Some(mut pending_protocol_input_events) = pending_protocol_input_events {
            let deferred = pending_protocol_input_events
                .drain()
                .filter(|event| !matches!(event.action, BackendInputAction::PointerMoved { .. }))
                .collect();
            pending_protocol_input_events.replace(deferred);
        }
    }

    for event in pointer_events.read() {
        let current = (event.x, event.y);
        let previous = gesture.last_pointer.replace(current);

        if !combo_active {
            continue;
        }

        if gesture.active_output.is_none() {
            gesture.active_output =
                focused_output.name.as_ref().map(|name| OutputName::from(name.clone()));
        }

        let Some(output_name) = gesture.active_output.clone() else {
            continue;
        };
        let Some((previous_x, previous_y)) = previous else {
            continue;
        };

        gesture.engaged = true;

        let delta_x = current.0 - previous_x + gesture.remainder_x;
        let delta_y = current.1 - previous_y + gesture.remainder_y;
        let pan_x = delta_x.trunc() as isize;
        let pan_y = delta_y.trunc() as isize;

        gesture.remainder_x = delta_x - pan_x as f64;
        gesture.remainder_y = delta_y - pan_y as f64;

        if pan_x == 0 && pan_y == 0 {
            continue;
        }

        pending_output_controls.named(output_name).pan_viewport_by(pan_x, pan_y);
        pending_input_events.push(nekoland_ecs::resources::InputEventRecord {
            source: "pointer:viewport".to_owned(),
            detail: format!("panned viewport by ({pan_x}, {pan_y})"),
        });
    }

    viewport_pan.active = combo_active && gesture.engaged;
}

#[cfg(test)]
mod tests {
    use bevy_ecs::message::Messages;
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::RunSystemOnce;
    use nekoland_ecs::events::PointerMotion;
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, CompositorConfig, FocusedOutputState, ModifierMask,
        ModifierState, PendingInputEvents, PendingOutputControls, PendingProtocolInputEvents,
        ViewportPointerPanState,
    };
    use nekoland_ecs::selectors::{OutputName, OutputSelector};

    use super::viewport_pointer_pan_system;

    #[test]
    fn viewport_pointer_pan_targets_locked_output_and_filters_protocol_motion() {
        let mut world = World::default();
        world.insert_resource(CompositorConfig::default());
        world.init_resource::<ModifierState>();
        world.init_resource::<PendingOutputControls>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<ViewportPointerPanState>();
        world.init_resource::<Messages<PointerMotion>>();
        world.insert_resource(FocusedOutputState { name: Some("DP-1".to_owned()) });
        world.insert_resource(PendingProtocolInputEvents::from_items(vec![BackendInputEvent {
            device: "winit".to_owned(),
            action: BackendInputAction::PointerMoved { x: 24.0, y: 17.0 },
        }]));

        {
            let mut modifiers = world.resource_mut::<ModifierState>();
            modifiers.logo = true;
            modifiers.alt = true;
        }

        world.write_message(PointerMotion { x: 20.0, y: 10.0 });
        world.run_system_once(viewport_pointer_pan_system).expect("viewport pan system should run");
        world.resource_mut::<PendingOutputControls>().clear();
        world.resource_mut::<PendingInputEvents>().clear();
        world.insert_resource(PendingProtocolInputEvents::from_items(vec![BackendInputEvent {
            device: "winit".to_owned(),
            action: BackendInputAction::PointerMoved { x: 24.0, y: 17.0 },
        }]));

        world.write_message(PointerMotion { x: 24.0, y: 17.0 });
        world.run_system_once(viewport_pointer_pan_system).expect("viewport pan system should run");

        let output_controls = world.resource::<PendingOutputControls>();
        let protocol_inputs = world.resource::<PendingProtocolInputEvents>();
        let input_events = world.resource::<PendingInputEvents>();
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
        assert!(
            protocol_inputs.is_empty(),
            "viewport pan gesture should swallow protocol pointer motion"
        );
        assert!(
            input_events.iter().any(|event| event.detail.contains("panned viewport by (4, 7)"))
        );
        assert!(viewport_pan.active, "drag state should stay active while modifiers are held");
    }

    #[test]
    fn viewport_pointer_pan_uses_configured_modifier_mask() {
        let mut world = World::default();
        let mut config = CompositorConfig::default();
        config.viewport_pan_modifiers = ModifierMask::new(true, false, true, false);
        world.insert_resource(config);
        world.init_resource::<ModifierState>();
        world.init_resource::<PendingOutputControls>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<ViewportPointerPanState>();
        world.init_resource::<Messages<PointerMotion>>();
        world.insert_resource(FocusedOutputState { name: Some("DP-1".to_owned()) });

        {
            let mut modifiers = world.resource_mut::<ModifierState>();
            modifiers.ctrl = true;
            modifiers.shift = true;
        }

        world.write_message(PointerMotion { x: 20.0, y: 10.0 });
        world.run_system_once(viewport_pointer_pan_system).expect("viewport pan system should run");
        world.resource_mut::<PendingOutputControls>().clear();

        world.write_message(PointerMotion { x: 24.0, y: 17.0 });
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
}
