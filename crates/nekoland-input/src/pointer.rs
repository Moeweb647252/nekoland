use bevy_ecs::message::MessageWriter;
use bevy_ecs::prelude::{Local, Res, ResMut};
use bevy_ecs::system::SystemParam;
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::events::{PointerButton, PointerMotion};
use nekoland_ecs::resources::{
    FocusedOutputState, GlobalPointerPosition, InputEventRecord, KeyShortcut, OutputSnapshotState,
    PendingInputEvents, PendingOutputControls, PhysicalPointerPosition, PlatformInputAction,
    PointerDelta, PressedKeys, ViewportPointerPanState, WaylandIngress,
};
use nekoland_ecs::selectors::OutputSelector;

#[derive(Debug, Default)]
pub(crate) struct ViewportPointerPanGestureState {
    active_output: Option<OutputSelector>,
    remainder_x: f64,
    remainder_y: f64,
    engaged: bool,
}

const POINTER_BOUNDS_EPSILON: f64 = 0.001;

#[derive(SystemParam)]
pub(crate) struct ViewportPointerPanParams<'w, 's> {
    pressed_keys: Res<'w, PressedKeys>,
    focused_output: Res<'w, FocusedOutputState>,
    pointer_delta: ResMut<'w, PointerDelta>,
    viewport_pan: ResMut<'w, ViewportPointerPanState>,
    pending_output_controls: ResMut<'w, PendingOutputControls>,
    pending_input_events: ResMut<'w, PendingInputEvents>,
    gesture: Local<'s, ViewportPointerPanGestureState>,
}

/// Consumes pointer-related backend input records into raw pointer state, pointer deltas, and
/// higher-level button messages.
pub fn pointer_input_system(
    wayland_ingress: Option<Res<'_, WaylandIngress>>,
    pointer: Res<GlobalPointerPosition>,
    mut physical_pointer: ResMut<PhysicalPointerPosition>,
    mut pointer_delta: ResMut<PointerDelta>,
    mut button_events: MessageWriter<PointerButton>,
    mut pending_input_events: ResMut<PendingInputEvents>,
) {
    pointer_delta.dx = 0.0;
    pointer_delta.dy = 0.0;
    let Some(wayland_ingress) = wayland_ingress else {
        return;
    };

    for event in wayland_ingress.platform_input_events.iter() {
        match &event.action {
            PlatformInputAction::PointerMoved { x, y } => {
                if physical_pointer.needs_resync {
                    physical_pointer.x = *x;
                    physical_pointer.y = *y;
                    physical_pointer.initialized = true;
                    physical_pointer.needs_resync = false;

                    pending_input_events.push(InputEventRecord {
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
                pointer_delta.dx += *x - previous_x;
                pointer_delta.dy += *y - previous_y;
                physical_pointer.x = *x;
                physical_pointer.y = *y;
                physical_pointer.initialized = true;
                physical_pointer.needs_resync = false;

                pending_input_events.push(InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("moved to ({x:.1}, {y:.1})"),
                });
            }
            PlatformInputAction::PointerDelta { dx, dy } => {
                pointer_delta.dx += *dx;
                pointer_delta.dy += *dy;
                physical_pointer.initialized = false;
                physical_pointer.needs_resync = true;

                pending_input_events.push(InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("delta ({dx:.1}, {dy:.1})"),
                });
            }
            PlatformInputAction::PointerButton { button_code, pressed } => {
                button_events.write(PointerButton { button_code: *button_code, pressed: *pressed });
                pending_input_events.push(InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!(
                        "button {button_code} {}",
                        if *pressed { "pressed" } else { "released" }
                    ),
                });
            }
            PlatformInputAction::PointerAxis { horizontal, vertical } => {
                pending_input_events.push(InputEventRecord {
                    source: format!("pointer:{}", event.device),
                    detail: format!("axis ({horizontal:.1}, {vertical:.1})"),
                });
            }
            PlatformInputAction::FocusChanged { focused } => {
                if !focused {
                    physical_pointer.initialized = false;
                    physical_pointer.needs_resync = false;
                    pointer_delta.dx = 0.0;
                    pointer_delta.dy = 0.0;
                }
            }
            _ => {}
        }
    }
}

/// Applies any unconsumed raw pointer delta to the logical cursor position shared by the rest of
/// the compositor.
pub fn cursor_motion_system(
    mut pointer: ResMut<GlobalPointerPosition>,
    focused_output: Option<Res<FocusedOutputState>>,
    wayland_ingress: Option<Res<WaylandIngress>>,
    mut pointer_delta: ResMut<PointerDelta>,
    mut motion_events: MessageWriter<PointerMotion>,
) {
    if pointer_delta.dx == 0.0 && pointer_delta.dy == 0.0 {
        return;
    }

    let next_x = pointer.x + pointer_delta.dx;
    let next_y = pointer.y + pointer_delta.dy;
    let (clamped_x, clamped_y) = wayland_ingress
        .as_deref()
        .map(|ingress| &ingress.output_snapshots)
        .and_then(|outputs| {
            clamp_pointer_to_active_output(
                (next_x, next_y),
                &pointer,
                focused_output.as_deref(),
                outputs,
            )
        })
        .unwrap_or((next_x, next_y));

    pointer.x = clamped_x;
    pointer.y = clamped_y;
    pointer_delta.dx = 0.0;
    pointer_delta.dy = 0.0;
    motion_events.write(PointerMotion { x: pointer.x, y: pointer.y });
}

pub fn focused_output_tracking_system(
    pointer: Res<GlobalPointerPosition>,
    mut focused_output: ResMut<FocusedOutputState>,
    wayland_ingress: Option<Res<WaylandIngress>>,
) {
    let Some(outputs) = wayland_ingress.as_deref().map(|ingress| &ingress.output_snapshots) else {
        return;
    };

    let next_output = outputs
        .outputs
        .iter()
        .find(|output| output.contains_point(pointer.x, pointer.y))
        .map(|output| Some(output.output_id))
        .or_else(|| {
            focused_output.id.and_then(|current| {
                outputs
                    .outputs
                    .iter()
                    .find(|output| output.output_id == current)
                    .map(|_| Some(current))
            })
        });

    let next_id = next_output.flatten();
    if focused_output.id != next_id {
        focused_output.id = next_id;
    }
}

fn clamp_pointer_to_active_output(
    next_pointer: (f64, f64),
    current_pointer: &GlobalPointerPosition,
    focused_output: Option<&FocusedOutputState>,
    outputs: &OutputSnapshotState,
) -> Option<(f64, f64)> {
    let output = focused_output
        .and_then(|focused_output| {
            focused_output.id.and_then(|output_id| {
                outputs.outputs.iter().find(|output| output.output_id == output_id)
            })
        })
        .or_else(|| {
            outputs.outputs.iter().find(|output| {
                pointer_within_output((current_pointer.x, current_pointer.y), output)
            })
        })
        .or_else(|| {
            outputs.outputs.iter().find(|output| pointer_within_output(next_pointer, output))
        })
        .or_else(|| outputs.outputs.first())?;

    let left = output.x as f64;
    let top = output.y as f64;
    let right = left + f64::from(output.width.max(1));
    let bottom = top + f64::from(output.height.max(1));

    Some((
        next_pointer.0.clamp(left, right - POINTER_BOUNDS_EPSILON),
        next_pointer.1.clamp(top, bottom - POINTER_BOUNDS_EPSILON),
    ))
}

fn pointer_within_output(
    pointer: (f64, f64),
    output: &nekoland_ecs::resources::OutputGeometrySnapshot,
) -> bool {
    output.contains_point(pointer.0, pointer.1)
}

/// Treats the configured viewport-pan shortcut plus pointer delta as direct viewport panning on
/// the focused output.
pub(crate) fn viewport_pointer_pan_system(
    config: Res<CompositorConfig>,
    pan: ViewportPointerPanParams<'_, '_>,
) {
    let ViewportPointerPanParams {
        pressed_keys,
        focused_output,
        mut pointer_delta,
        mut viewport_pan,
        mut pending_output_controls,
        mut pending_input_events,
        mut gesture,
    } = pan;

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
        gesture.active_output = focused_output.id.map(OutputSelector::Id);
    }

    let Some(output_selector) = gesture.active_output.clone() else {
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
    pending_output_controls.select(output_selector).pan_viewport_by(pan_x, pan_y);
    pending_input_events.push(InputEventRecord {
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
    use nekoland_config::resources::CompositorConfig;
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::events::{PointerButton, PointerMotion};
    use nekoland_ecs::resources::PendingInputEvents;
    use nekoland_ecs::resources::{
        BackendInputAction, BackendInputEvent, FocusedOutputState, GlobalPointerPosition,
        ModifierMask, OutputGeometrySnapshot, OutputSnapshotState, PendingOutputControls,
        PendingPlatformInputEvents, PhysicalPointerPosition, PointerDelta, PressedKeys,
        ViewportPointerPanState, WaylandIngress,
    };
    use nekoland_ecs::selectors::OutputSelector;

    use super::{
        cursor_motion_system, focused_output_tracking_system, pointer_input_system,
        viewport_pointer_pan_system,
    };

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
        world.insert_resource(FocusedOutputState { id: Some(OutputId(7)) });

        {
            let mut pressed_keys = world.resource_mut::<PressedKeys>();
            pressed_keys.record_key(133, true);
            pressed_keys.record_key(64, true);
        }

        let Ok(()) = world.run_system_once(viewport_pointer_pan_system) else {
            panic!("viewport pan system should run");
        };
        let Ok(()) = world.run_system_once(cursor_motion_system) else {
            panic!("cursor motion system should run");
        };

        let output_controls = world.resource::<PendingOutputControls>();
        let pointer = world.resource::<GlobalPointerPosition>();
        let viewport_pan = world.resource::<ViewportPointerPanState>();

        assert_eq!(
            output_controls.as_slice(),
            &[nekoland_ecs::resources::PendingOutputControl {
                selector: OutputSelector::Id(OutputId(7)),
                enabled: None,
                configuration: None,
                viewport_origin: None,
                viewport_pan: Some(nekoland_ecs::resources::OutputViewportPan {
                    delta_x: 4,
                    delta_y: 7,
                }),
                center_viewport_on: None,
                clear_overlays: false,
                overlay_updates: Vec::new(),
            }]
        );
        assert_eq!((pointer.x, pointer.y), (20.0, 10.0));
        assert!(viewport_pan.active);
    }

    #[test]
    fn viewport_pointer_pan_uses_configured_modifier_mask() {
        let mut world = World::default();
        let config = CompositorConfig {
            viewport_pan_modifiers: ModifierMask::new(true, false, true, false),
            ..CompositorConfig::default()
        };
        world.insert_resource(config);
        world.insert_resource(PointerDelta { dx: 4.0, dy: 7.0 });
        world.insert_resource(PressedKeys::default());
        world.init_resource::<PendingOutputControls>();
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<ViewportPointerPanState>();
        world.insert_resource(FocusedOutputState { id: Some(OutputId(7)) });

        {
            let mut pressed_keys = world.resource_mut::<PressedKeys>();
            pressed_keys.record_key(37, true);
            pressed_keys.record_key(50, true);
        }

        let Ok(()) = world.run_system_once(viewport_pointer_pan_system) else {
            panic!("viewport pan system should run");
        };

        let Some(output_controls) = world.get_resource::<PendingOutputControls>() else {
            panic!("output controls should exist");
        };
        let Some(viewport_pan) = world.get_resource::<ViewportPointerPanState>() else {
            panic!("viewport pan state should exist");
        };

        assert_eq!(
            output_controls.as_slice(),
            &[nekoland_ecs::resources::PendingOutputControl {
                selector: OutputSelector::Id(OutputId(7)),
                enabled: None,
                configuration: None,
                viewport_origin: None,
                viewport_pan: Some(nekoland_ecs::resources::OutputViewportPan {
                    delta_x: 4,
                    delta_y: 7,
                }),
                center_viewport_on: None,
                clear_overlays: false,
                overlay_updates: Vec::new(),
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

        let Ok(()) = world.run_system_once(cursor_motion_system) else {
            panic!("cursor motion system should run");
        };

        let pointer = world.resource::<GlobalPointerPosition>();
        let pointer_delta = world.resource::<PointerDelta>();
        assert_eq!((pointer.x, pointer.y), (24.0, 17.0));
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (0.0, 0.0));
    }

    #[test]
    fn cursor_motion_clamps_to_focused_output_bounds() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 90.0, y: 40.0 });
        world.insert_resource(PointerDelta { dx: 20.0, dy: 30.0 });
        world.insert_resource(FocusedOutputState { id: Some(OutputId(1)) });
        world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![OutputGeometrySnapshot {
                    output_id: OutputId(1),
                    name: "DP-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 50,
                    scale: 1,
                    refresh_millihz: 60_000,
                }],
            },
            ..WaylandIngress::default()
        });
        world.init_resource::<Messages<PointerMotion>>();

        let Ok(()) = world.run_system_once(cursor_motion_system) else {
            panic!("cursor motion system should run");
        };

        let pointer = world.resource::<GlobalPointerPosition>();
        assert!((pointer.x - 99.999).abs() < 0.01, "pointer.x should stay inside output bounds");
        assert!((pointer.y - 49.999).abs() < 0.01, "pointer.y should stay inside output bounds");
    }

    #[test]
    fn focused_output_tracking_uses_output_snapshot_state() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 150.0, y: 40.0 });
        world.insert_resource(FocusedOutputState::default());
        world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![
                    OutputGeometrySnapshot {
                        output_id: OutputId(1),
                        name: "HDMI-A-1".to_owned(),
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 80,
                        scale: 1,
                        refresh_millihz: 60_000,
                    },
                    OutputGeometrySnapshot {
                        output_id: OutputId(2),
                        name: "DP-1".to_owned(),
                        x: 100,
                        y: 0,
                        width: 100,
                        height: 80,
                        scale: 1,
                        refresh_millihz: 60_000,
                    },
                ],
            },
            ..WaylandIngress::default()
        });

        let Ok(()) = world.run_system_once(focused_output_tracking_system) else {
            panic!("focused output tracking system should run");
        };

        assert_eq!(world.resource::<FocusedOutputState>().id, Some(OutputId(2)));
    }

    #[test]
    fn pointer_input_resyncs_after_relative_motion_without_jumping() {
        let mut world = World::default();
        world.insert_resource(GlobalPointerPosition { x: 20.0, y: 10.0 });
        world.insert_resource(PhysicalPointerPosition::default());
        world.insert_resource(PointerDelta::default());
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<Messages<PointerButton>>();
        world.init_resource::<Messages<PointerMotion>>();
        world.insert_resource(WaylandIngress {
            platform_input_events: PendingPlatformInputEvents::from_items(vec![
                BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::PointerDelta { dx: 4.0, dy: 7.0 },
                },
            ]),
            ..WaylandIngress::default()
        });
        let Ok(()) = world.run_system_once(pointer_input_system) else {
            panic!("pointer input system should run");
        };
        let Ok(()) = world.run_system_once(cursor_motion_system) else {
            panic!("cursor motion system should run");
        };

        let pointer = world.resource::<GlobalPointerPosition>();
        let physical = world.resource::<PhysicalPointerPosition>();
        assert_eq!((pointer.x, pointer.y), (24.0, 17.0));
        assert!(!physical.initialized);
        assert!(physical.needs_resync);

        world.resource_mut::<WaylandIngress>().platform_input_events =
            PendingPlatformInputEvents::from_items(vec![BackendInputEvent {
                device: "winit".to_owned(),
                action: BackendInputAction::PointerMoved { x: 300.0, y: 200.0 },
            }]);
        let Ok(()) = world.run_system_once(pointer_input_system) else {
            panic!("pointer input system should run");
        };

        let pointer_delta = world.resource::<PointerDelta>();
        let physical = world.resource::<PhysicalPointerPosition>();
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (0.0, 0.0));
        assert_eq!((physical.x, physical.y), (300.0, 200.0));
        assert!(physical.initialized);
        assert!(!physical.needs_resync);

        world.resource_mut::<WaylandIngress>().platform_input_events =
            PendingPlatformInputEvents::from_items(vec![BackendInputEvent {
                device: "winit".to_owned(),
                action: BackendInputAction::PointerMoved { x: 302.0, y: 203.0 },
            }]);
        let Ok(()) = world.run_system_once(pointer_input_system) else {
            panic!("pointer input system should run");
        };

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
        world.init_resource::<PendingInputEvents>();
        world.init_resource::<Messages<PointerButton>>();
        world.insert_resource(WaylandIngress {
            platform_input_events: PendingPlatformInputEvents::from_items(vec![
                BackendInputEvent {
                    device: "winit".to_owned(),
                    action: BackendInputAction::FocusChanged { focused: false },
                },
            ]),
            ..WaylandIngress::default()
        });
        let Ok(()) = world.run_system_once(pointer_input_system) else {
            panic!("pointer input system should run");
        };

        let physical = world.resource::<PhysicalPointerPosition>();
        let pointer_delta = world.resource::<PointerDelta>();
        let ingress = world.resource::<WaylandIngress>();

        assert!(!physical.initialized);
        assert!(!physical.needs_resync);
        assert_eq!((pointer_delta.dx, pointer_delta.dy), (0.0, 0.0));
        assert_eq!(
            ingress.platform_input_events.as_slice(),
            &[BackendInputEvent {
                device: "winit".to_owned(),
                action: BackendInputAction::FocusChanged { focused: false },
            }]
        );
    }
}
