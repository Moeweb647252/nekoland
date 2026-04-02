use std::collections::BTreeSet;
use std::marker::PhantomData;

use bevy_ecs::prelude::{Local, NonSendMut, Res, ResMut, Resource};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::resources::{
    BackendInputAction, CompiledShortcutMap, KeyShortcut, PendingProtocolInputEvents, PressedKeys,
    ShortcutTrigger,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PointerSurfaceFocus {
    pub(crate) surface_id: u64,
    pub(crate) surface_origin: super::Point<f64, super::Logical>,
}

#[derive(Debug, Clone, Copy)]
struct GlobalSurfaceBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl GlobalSurfaceBounds {
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SeatInputSyncState {
    pub(crate) initialized: bool,
    pub(crate) host_focused: bool,
    pub(crate) keyboard_focus: Option<u64>,
    pub(crate) pointer_focus: Option<u64>,
    pub(crate) pointer_location: super::Point<f64, super::Logical>,
}

impl Default for SeatInputSyncState {
    fn default() -> Self {
        Self {
            initialized: false,
            host_focused: true,
            keyboard_focus: None,
            pointer_focus: None,
            pointer_location: (0.0, 0.0).into(),
        }
    }
}

#[derive(Debug, Default, Resource)]
pub(crate) struct ShortcutInputFilterState {
    pressed_keys: PressedKeys,
    consumed_keys: BTreeSet<u32>,
}

#[derive(SystemParam)]
pub(crate) struct DispatchSeatInputParams<'w, 's> {
    pub(crate) clock: Option<Res<'w, nekoland_ecs::resources::CompositorClock>>,
    pub(crate) keyboard_focus: Option<Res<'w, nekoland_ecs::resources::KeyboardFocusState>>,
    pub(crate) pointer: Option<Res<'w, nekoland_ecs::resources::GlobalPointerPosition>>,
    pub(crate) render_plan: Option<Res<'w, nekoland_ecs::resources::RenderPlan>>,
    pub(crate) surface_presentation:
        Option<Res<'w, nekoland_ecs::resources::SurfacePresentationSnapshot>>,
    pub(crate) pending_protocol_input_events:
        ResMut<'w, nekoland_ecs::resources::PendingProtocolInputEvents>,
    pub(crate) output_snapshots: Option<Res<'w, nekoland_ecs::resources::OutputSnapshotState>>,
    pub(crate) _marker: PhantomData<&'s ()>,
}

pub(crate) struct PointerFocusInputs<'a> {
    pub(crate) render_plan: Option<&'a nekoland_ecs::resources::RenderPlan>,
    pub(crate) surface_presentation:
        Option<&'a nekoland_ecs::resources::SurfacePresentationSnapshot>,
    pub(crate) output_snapshots: Option<&'a nekoland_ecs::resources::OutputSnapshotState>,
}

pub(crate) fn filter_consumed_shortcut_input_system(
    compiled_shortcuts: Res<'_, CompiledShortcutMap>,
    mut pending_protocol_input_events: ResMut<'_, PendingProtocolInputEvents>,
    mut filter: ResMut<'_, ShortcutInputFilterState>,
) {
    if compiled_shortcuts.is_empty() {
        return;
    }

    let mut filtered_events = Vec::with_capacity(pending_protocol_input_events.len());
    for event in pending_protocol_input_events.take() {
        let keep = match &event.action {
            BackendInputAction::FocusChanged { focused } => {
                if !focused {
                    filter.pressed_keys.reset_all();
                    filter.consumed_keys.clear();
                }
                true
            }
            BackendInputAction::Key { keycode, pressed } => {
                !should_consume_shortcut_key_event(
                    *keycode,
                    *pressed,
                    &compiled_shortcuts,
                    &mut filter,
                )
            }
            _ => true,
        };
        if keep {
            filtered_events.push(event);
        }
    }
    pending_protocol_input_events.replace(filtered_events);
}

pub(crate) fn dispatch_seat_input_system(
    params: DispatchSeatInputParams<'_, '_>,
    mut seat_sync: Local<'_, SeatInputSyncState>,
    server: Option<NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    if !seat_sync.initialized {
        seat_sync.initialized = true;
        seat_sync.host_focused = true;
    }

    let DispatchSeatInputParams {
        clock,
        keyboard_focus,
        pointer,
        render_plan,
        surface_presentation,
        mut pending_protocol_input_events,
        output_snapshots,
        ..
    } = params;
    let time = clock.as_deref().map_or(0, compositor_time_millis);
    let keyboard_focus = keyboard_focus.as_deref();
    let pointer = pointer.as_deref();
    let focus_inputs = PointerFocusInputs {
        render_plan: render_plan.as_deref(),
        surface_presentation: surface_presentation.as_deref(),
        output_snapshots: output_snapshots.as_deref(),
    };

    for event in pending_protocol_input_events.drain() {
        sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);

        match event.action {
            nekoland_ecs::resources::BackendInputAction::FocusChanged { focused } => {
                seat_sync.host_focused = focused;
                sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    &focus_inputs,
                    time,
                );
            }
            nekoland_ecs::resources::BackendInputAction::Key { keycode, pressed } => {
                sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);
                if seat_sync.host_focused {
                    server.dispatch_keyboard_input(keycode, pressed, time);
                }
            }
            nekoland_ecs::resources::BackendInputAction::PointerMoved { .. }
            | nekoland_ecs::resources::BackendInputAction::PointerDelta { .. } => {
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    &focus_inputs,
                    time,
                );
            }
            nekoland_ecs::resources::BackendInputAction::PointerButton { button_code, pressed } => {
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    &focus_inputs,
                    time,
                );
                if seat_sync.host_focused {
                    server.dispatch_pointer_button(
                        button_code,
                        pressed,
                        time,
                        seat_sync.pointer_focus,
                    );
                }
            }
            nekoland_ecs::resources::BackendInputAction::PointerAxis { horizontal, vertical } => {
                sync_pointer_focus_if_needed(
                    &mut server,
                    &mut seat_sync,
                    pointer,
                    &focus_inputs,
                    time,
                );
                if seat_sync.host_focused {
                    server.dispatch_pointer_axis(horizontal, vertical, time);
                }
            }
        }
    }

    sync_keyboard_focus_if_needed(&mut server, &mut seat_sync, keyboard_focus);
    sync_pointer_focus_if_needed(&mut server, &mut seat_sync, pointer, &focus_inputs, time);
}

pub(crate) fn compositor_time_millis(clock: &nekoland_ecs::resources::CompositorClock) -> u32 {
    clock.uptime_millis.min(u128::from(u32::MAX)) as u32
}

fn should_consume_shortcut_key_event(
    keycode: u32,
    pressed: bool,
    compiled_shortcuts: &CompiledShortcutMap,
    filter: &mut ShortcutInputFilterState,
) -> bool {
    let previous_pressed = filter.pressed_keys.clone();
    let mut next_pressed = previous_pressed.clone();
    next_pressed.clear_frame_transitions();
    next_pressed.record_key(keycode, pressed);

    let transition_consumed = compiled_shortcuts.iter().any(|shortcut| {
        let previous_active = shortcut_active(&previous_pressed, &shortcut.combo);
        let active = shortcut_active(&next_pressed, &shortcut.combo);
        match shortcut.trigger {
            ShortcutTrigger::Press => press_triggered(&next_pressed, &shortcut.combo, previous_active),
            ShortcutTrigger::Release => {
                release_triggered(&next_pressed, &shortcut.combo, previous_active)
            }
            ShortcutTrigger::Hold => active != previous_active,
        }
    });

    let suppress = (!pressed && filter.consumed_keys.contains(&keycode)) || transition_consumed;

    if pressed {
        if suppress {
            filter.consumed_keys.insert(keycode);
        }
    } else {
        filter.consumed_keys.remove(&keycode);
    }
    filter.pressed_keys = next_pressed;

    suppress
}

fn shortcut_active(pressed_keys: &PressedKeys, combo: &KeyShortcut) -> bool {
    if combo.keycode.is_none() {
        combo.modifiers.matches_required(pressed_keys.modifiers())
    } else {
        pressed_keys.is_pressed(combo)
    }
}

fn press_triggered(
    pressed_keys: &PressedKeys,
    combo: &KeyShortcut,
    previous_active: bool,
) -> bool {
    if combo.keycode.is_none() {
        shortcut_active(pressed_keys, combo) && !previous_active
    } else {
        pressed_keys.just_pressed(combo)
    }
}

fn release_triggered(
    pressed_keys: &PressedKeys,
    combo: &KeyShortcut,
    previous_active: bool,
) -> bool {
    if combo.keycode.is_none() {
        !shortcut_active(pressed_keys, combo) && previous_active
    } else {
        pressed_keys.just_released(combo)
    }
}

fn sync_keyboard_focus_if_needed(
    server: &mut super::server::SmithayProtocolServer,
    seat_sync: &mut SeatInputSyncState,
    keyboard_focus: Option<&nekoland_ecs::resources::KeyboardFocusState>,
) {
    let desired_focus = seat_sync
        .host_focused
        .then(|| keyboard_focus.and_then(|focus| focus.focused_surface))
        .flatten();

    if seat_sync.keyboard_focus == desired_focus {
        return;
    }

    server.sync_keyboard_focus(desired_focus);
    seat_sync.keyboard_focus = desired_focus;
}

fn sync_pointer_focus_if_needed(
    server: &mut super::server::SmithayProtocolServer,
    seat_sync: &mut SeatInputSyncState,
    pointer: Option<&nekoland_ecs::resources::GlobalPointerPosition>,
    focus_inputs: &PointerFocusInputs<'_>,
    time: u32,
) {
    let location = pointer
        .map(|pointer| super::Point::<f64, super::Logical>::from((pointer.x, pointer.y)))
        .unwrap_or(seat_sync.pointer_location);
    let desired_focus = if seat_sync.host_focused {
        pointer.and_then(|pointer| {
            pointer_focus_target(pointer.x, pointer.y, Some(&*server), location, focus_inputs)
        })
    } else {
        None
    };
    let desired_focus_id = desired_focus.map(|focus| focus.surface_id);

    if seat_sync.pointer_focus == desired_focus_id && seat_sync.pointer_location == location {
        return;
    }

    server.dispatch_pointer_motion(desired_focus, location, time);
    seat_sync.pointer_focus = desired_focus_id;
    seat_sync.pointer_location = location;
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::RunSystemOnce;
    use nekoland_ecs::resources::{
        BackendInputEvent, CompiledShortcut, CompiledShortcutMap, KeyShortcut, ModifierMask,
        PendingProtocolInputEvents, ShortcutTrigger,
    };

    use super::{ShortcutInputFilterState, filter_consumed_shortcut_input_system};

    fn compiled_shortcut(
        id: &str,
        binding: &str,
        combo: KeyShortcut,
        trigger: ShortcutTrigger,
    ) -> CompiledShortcut {
        CompiledShortcut {
            id: id.to_owned(),
            owner: "test".to_owned(),
            description: id.to_owned(),
            binding: binding.to_owned(),
            combo,
            trigger,
            overridden: false,
        }
    }

    #[test]
    fn shortcut_filter_suppresses_consumed_shortcut_key_events() {
        let mut world = World::new();
        let mut compiled = CompiledShortcutMap::default();
        compiled.replace(
            [
                (
                    "window_switcher.hold".to_owned(),
                    compiled_shortcut(
                        "window_switcher.hold",
                        "Alt",
                        KeyShortcut::modifier_only(ModifierMask::new(false, true, false, false)),
                        ShortcutTrigger::Hold,
                    ),
                ),
                (
                    "window_switcher.cycle_next".to_owned(),
                    compiled_shortcut(
                        "window_switcher.cycle_next",
                        "Alt+Tab",
                        KeyShortcut::new(ModifierMask::new(false, true, false, false), Some(23)),
                        ShortcutTrigger::Press,
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        );
        world.insert_resource(compiled);
        world.insert_resource(ShortcutInputFilterState::default());
        world.insert_resource(PendingProtocolInputEvents::from_items(vec![
            BackendInputEvent {
                device: "test".to_owned(),
                action: nekoland_ecs::resources::BackendInputAction::Key {
                    keycode: 64,
                    pressed: true,
                },
            },
            BackendInputEvent {
                device: "test".to_owned(),
                action: nekoland_ecs::resources::BackendInputAction::Key {
                    keycode: 23,
                    pressed: true,
                },
            },
            BackendInputEvent {
                device: "test".to_owned(),
                action: nekoland_ecs::resources::BackendInputAction::Key {
                    keycode: 23,
                    pressed: false,
                },
            },
            BackendInputEvent {
                device: "test".to_owned(),
                action: nekoland_ecs::resources::BackendInputAction::Key {
                    keycode: 64,
                    pressed: false,
                },
            },
        ]));

        world
            .run_system_once(filter_consumed_shortcut_input_system)
            .expect("shortcut filter system should run");

        assert!(
            world.resource::<PendingProtocolInputEvents>().is_empty(),
            "shortcut-triggering key events should not be forwarded to clients"
        );
    }

    #[test]
    fn shortcut_filter_leaves_non_shortcut_key_events_intact() {
        let mut world = World::new();
        world.insert_resource(CompiledShortcutMap::default());
        world.insert_resource(ShortcutInputFilterState::default());
        world.insert_resource(PendingProtocolInputEvents::from_items(vec![BackendInputEvent {
            device: "test".to_owned(),
            action: nekoland_ecs::resources::BackendInputAction::Key {
                keycode: 38,
                pressed: true,
            },
        }]));

        world
            .run_system_once(filter_consumed_shortcut_input_system)
            .expect("shortcut filter system should run");

        assert_eq!(world.resource::<PendingProtocolInputEvents>().len(), 1);
    }
}

pub(crate) fn pointer_focus_target(
    pointer_x: f64,
    pointer_y: f64,
    server: Option<&super::server::SmithayProtocolServer>,
    location: super::Point<f64, super::Logical>,
    focus_inputs: &PointerFocusInputs<'_>,
) -> Option<PointerSurfaceFocus> {
    let render_plan = focus_inputs.render_plan?;
    let output_contexts = focus_inputs
        .output_snapshots?
        .outputs
        .iter()
        .map(|output| (output.output_id, output.x, output.y))
        .collect::<Vec<_>>();
    if let Some(surface_presentation) = focus_inputs.surface_presentation {
        for (output_id, placement_x, placement_y) in &output_contexts {
            let Some(output_plan) = render_plan.outputs.get(output_id) else { continue };
            for item_id in output_plan.ordered_item_ids().iter().rev() {
                let Some(item) = output_plan.item(*item_id) else {
                    continue;
                };
                let nekoland_ecs::resources::RenderPlanItem::Surface(item) = item else {
                    continue;
                };
                let Some(state) = surface_presentation.surfaces.get(&item.surface_id) else {
                    continue;
                };
                if !state.visible || !state.input_enabled {
                    continue;
                }
                let surface_origin = super::Point::<f64, super::Logical>::from((
                    f64::from(*placement_x + item.instance.rect.x),
                    f64::from(*placement_y + item.instance.rect.y),
                ));
                let accepted = if let Some(server) = server {
                    server.pointer_focus_candidate_accepts(
                        item.surface_id,
                        location,
                        surface_origin,
                    )
                } else {
                    global_surface_bounds_for_item(*placement_x, *placement_y, item.instance)
                        .is_some_and(|bounds| bounds.contains(pointer_x, pointer_y))
                };
                if !accepted {
                    continue;
                }
                return Some(PointerSurfaceFocus { surface_id: item.surface_id, surface_origin });
            }
        }

        return None;
    }
    for (output_id, placement_x, placement_y) in &output_contexts {
        let Some(output_plan) = render_plan.outputs.get(output_id) else { continue };
        for item_id in output_plan.ordered_item_ids().iter().rev() {
            let Some(item) = output_plan.item(*item_id) else {
                continue;
            };
            let nekoland_ecs::resources::RenderPlanItem::Surface(item) = item else {
                continue;
            };
            let surface_origin = super::Point::<f64, super::Logical>::from((
                f64::from(*placement_x + item.instance.rect.x),
                f64::from(*placement_y + item.instance.rect.y),
            ));
            let accepted = if let Some(server) = server {
                server.pointer_focus_candidate_accepts(item.surface_id, location, surface_origin)
            } else {
                global_surface_bounds_for_item(*placement_x, *placement_y, item.instance)
                    .is_some_and(|bounds| bounds.contains(pointer_x, pointer_y))
            };
            if !accepted {
                continue;
            }
            return Some(PointerSurfaceFocus { surface_id: item.surface_id, surface_origin });
        }
    }

    None
}

fn global_surface_bounds_for_item(
    placement_x: i32,
    placement_y: i32,
    instance: nekoland_ecs::resources::RenderItemInstance,
) -> Option<GlobalSurfaceBounds> {
    let rect = instance.visible_rect()?;
    Some(GlobalSurfaceBounds {
        x: f64::from(placement_x + rect.x),
        y: f64::from(placement_y + rect.y),
        width: f64::from(rect.width),
        height: f64::from(rect.height),
    })
}

impl super::FractionalScaleHandler for super::server::ProtocolRuntimeState {
    fn new_fractional_scale(&mut self, surface: super::WlSurface) {
        self.update_surface_fractional_scale(&surface);
    }
}

impl super::SeatHandler for super::server::ProtocolRuntimeState {
    type KeyboardFocus = super::WlSurface;
    type PointerFocus = super::WlSurface;
    type TouchFocus = super::WlSurface;

    fn seat_state(&mut self) -> &mut super::SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &super::Seat<Self>, _focused: Option<&super::WlSurface>) {}

    fn cursor_image(
        &mut self,
        _seat: &super::Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor_state.image = match image {
            smithay::input::pointer::CursorImageStatus::Hidden => {
                super::server::ProtocolCursorImage::Hidden
            }
            smithay::input::pointer::CursorImageStatus::Named(icon) => {
                super::server::ProtocolCursorImage::Named(icon)
            }
            smithay::input::pointer::CursorImageStatus::Surface(surface) => {
                let hotspot = super::compositor::with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<smithay::input::pointer::CursorImageSurfaceData>()
                        .and_then(|attributes| match attributes.lock() {
                            Ok(attributes) => Some(attributes.hotspot),
                            Err(_) => {
                                tracing::warn!("failed to lock cursor image surface attributes");
                                None
                            }
                        })
                        .unwrap_or_default()
                });
                super::server::ProtocolCursorImage::Surface {
                    surface,
                    hotspot_x: hotspot.x,
                    hotspot_y: hotspot.y,
                }
            }
        };
    }
}
