use std::cell::RefCell;
use std::rc::Rc;

use bevy_app::App;
use nekoland_config::resources::CompositorConfig;
use nekoland_core::calloop::with_wayland_calloop_registry;
use nekoland_core::error::NekolandError;
use nekoland_ecs::components::OutputProperties;
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, PendingBackendInputEvents, PendingProtocolInputEvents,
};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event as InputEventDevice, InputEvent, KeyState,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::reexports::input::{Device as LibinputDevice, Libinput};

use super::session::{DrmSessionStatus, SharedDrmSessionState};

#[derive(Debug, Clone, Copy, PartialEq)]
struct DrmInputBounds {
    width: f64,
    height: f64,
}

impl Default for DrmInputBounds {
    fn default() -> Self {
        Self { width: 1920.0, height: 1080.0 }
    }
}

#[derive(Debug, Default)]
pub struct DrmInputState {
    pub pending_input_events: Vec<BackendInputEvent>,
    bounds: DrmInputBounds,
    pointer_x: f64,
    pointer_y: f64,
}

pub type SharedDrmInputState = Rc<RefCell<DrmInputState>>;

pub(crate) fn install_drm_input_source(
    app: &mut App,
    session_state: SharedDrmSessionState,
    input_state: SharedDrmInputState,
) {
    with_wayland_calloop_registry(app, |registry| {
        registry.push(move |handle| {
            let (session, seat_name) = {
                let session_state = session_state.borrow();
                match &session_state.status {
                    DrmSessionStatus::Ready => {
                        let session = session_state.session.clone().ok_or_else(|| {
                            NekolandError::Runtime(
                                "drm session installed without an active libseat handle".to_owned(),
                            )
                        })?;
                        (session, session_state.seat_name.clone())
                    }
                    DrmSessionStatus::Failed(error) => {
                        return Err(NekolandError::Runtime(error.clone()));
                    }
                    DrmSessionStatus::Uninitialized => {
                        return Err(NekolandError::Runtime(
                            "drm tty session did not initialize before libinput setup".to_owned(),
                        ));
                    }
                }
            };

            let mut context = Libinput::new_with_udev(LibinputSessionInterface::from(session));
            context.udev_assign_seat(&seat_name).map_err(|_| {
                NekolandError::Runtime(format!("failed to assign libinput to seat {seat_name}"))
            })?;

            let session_state_for_events = session_state.clone();
            let input_state_for_events = input_state.clone();
            handle
                .insert_source(LibinputInputBackend::new(context), move |event, _, _| {
                    if !session_state_for_events.borrow().active {
                        return;
                    }

                    let mut input_state = input_state_for_events.borrow_mut();
                    if let Some(event) = translate_libinput_event(event, &mut input_state) {
                        input_state.pending_input_events.push(event);
                    }
                })
                .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

            Ok(())
        });
    });
}

pub(crate) fn drain_drm_input(
    config: Option<&CompositorConfig>,
    outputs: impl IntoIterator<Item = OutputProperties>,
    session_state: &SharedDrmSessionState,
    input_state: &SharedDrmInputState,
    pending_backend_inputs: &mut PendingBackendInputEvents,
    pending_protocol_inputs: &mut PendingProtocolInputEvents,
) {
    let active = session_state.borrow().active;
    let bounds = effective_input_bounds(outputs, config);

    let mut input_state = input_state.borrow_mut();
    input_state.bounds = bounds;

    if !active {
        input_state.pending_input_events.clear();
        return;
    }

    let pending_events = input_state.pending_input_events.drain(..).collect::<Vec<_>>();
    drop(input_state);

    pending_backend_inputs.extend(pending_events.iter().cloned());
    pending_protocol_inputs.extend(pending_events);
}

fn translate_libinput_event(
    input_event: InputEvent<LibinputInputBackend>,
    input_state: &mut DrmInputState,
) -> Option<BackendInputEvent> {
    match input_event {
        InputEvent::Keyboard { event } => Some(BackendInputEvent {
            device: libinput_device_id(&event.device()),
            action: BackendInputAction::Key {
                keycode: event.key_code().into(),
                pressed: event.state() == KeyState::Pressed,
            },
        }),
        InputEvent::PointerMotion { event } => {
            let (x, y) = input_state.apply_relative_motion(event.delta_x(), event.delta_y());
            Some(BackendInputEvent {
                device: libinput_device_id(&event.device()),
                action: BackendInputAction::PointerMoved { x, y },
            })
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let width = input_state.bounds.width.round().clamp(1.0, i32::MAX as f64) as i32;
            let height = input_state.bounds.height.round().clamp(1.0, i32::MAX as f64) as i32;
            let (x, y) = input_state
                .apply_absolute_motion(event.x_transformed(width), event.y_transformed(height));
            Some(BackendInputEvent {
                device: libinput_device_id(&event.device()),
                action: BackendInputAction::PointerMoved { x, y },
            })
        }
        InputEvent::PointerButton { event } => Some(BackendInputEvent {
            device: libinput_device_id(&event.device()),
            action: BackendInputAction::PointerButton {
                button_code: event.button_code(),
                pressed: event.state() == ButtonState::Pressed,
            },
        }),
        InputEvent::PointerAxis { event } => Some(BackendInputEvent {
            device: libinput_device_id(&event.device()),
            action: BackendInputAction::PointerAxis {
                horizontal: event
                    .amount(Axis::Horizontal)
                    .or_else(|| event.amount_v120(Axis::Horizontal))
                    .unwrap_or(0.0),
                vertical: event
                    .amount(Axis::Vertical)
                    .or_else(|| event.amount_v120(Axis::Vertical))
                    .unwrap_or(0.0),
            },
        }),
        _ => None,
    }
}

impl DrmInputState {
    fn apply_relative_motion(&mut self, dx: f64, dy: f64) -> (f64, f64) {
        self.pointer_x = clamp_pointer_coordinate(self.pointer_x + dx, self.bounds.width);
        self.pointer_y = clamp_pointer_coordinate(self.pointer_y + dy, self.bounds.height);
        (self.pointer_x, self.pointer_y)
    }

    fn apply_absolute_motion(&mut self, x: f64, y: f64) -> (f64, f64) {
        self.pointer_x = clamp_pointer_coordinate(x, self.bounds.width);
        self.pointer_y = clamp_pointer_coordinate(y, self.bounds.height);
        (self.pointer_x, self.pointer_y)
    }
}

fn effective_input_bounds(
    outputs: impl IntoIterator<Item = OutputProperties>,
    config: Option<&CompositorConfig>,
) -> DrmInputBounds {
    live_output_bounds(outputs)
        .or_else(|| config.and_then(configured_output_bounds))
        .unwrap_or_default()
}

fn live_output_bounds(
    outputs: impl IntoIterator<Item = OutputProperties>,
) -> Option<DrmInputBounds> {
    let mut width = 0_u32;
    let mut height = 0_u32;

    for properties in outputs {
        width = width.max(properties.width.max(1));
        height = height.max(properties.height.max(1));
    }

    if width == 0 || height == 0 {
        return None;
    }

    Some(DrmInputBounds { width: f64::from(width), height: f64::from(height) })
}

fn configured_output_bounds(config: &CompositorConfig) -> Option<DrmInputBounds> {
    let mut width = 0_u32;
    let mut height = 0_u32;

    for output in config.outputs.iter().filter(|output| output.enabled) {
        let Some((dimensions, _)) = output.mode.split_once('@') else { continue };
        let Some((output_width, output_height)) = dimensions.split_once('x') else { continue };
        let Ok(output_width) = output_width.parse::<u32>() else { continue };
        let Ok(output_height) = output_height.parse::<u32>() else { continue };
        width = width.max(output_width.max(1));
        height = height.max(output_height.max(1));
    }

    if width == 0 || height == 0 {
        return None;
    }

    Some(DrmInputBounds { width: f64::from(width), height: f64::from(height) })
}

fn clamp_pointer_coordinate(value: f64, extent: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, (extent.max(1.0) - 1.0).max(0.0))
}

fn libinput_device_id(device: &LibinputDevice) -> String {
    let name = device.name().trim();
    if name.is_empty() { "libinput".to_owned() } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
    use nekoland_config::resources::{CompositorConfig, ConfiguredOutput};
    use nekoland_ecs::components::OutputProperties;

    use super::{DrmInputState, configured_output_bounds, live_output_bounds};

    #[test]
    fn configured_output_bounds_follow_enabled_output_mode() {
        let config = CompositorConfig {
            outputs: vec![
                ConfiguredOutput {
                    name: "DP-1".to_owned(),
                    mode: "1920x1080@60".to_owned(),
                    scale: 1,
                    enabled: true,
                },
                ConfiguredOutput {
                    name: "HDMI-A-1".to_owned(),
                    mode: "1280x720@60".to_owned(),
                    scale: 1,
                    enabled: false,
                },
            ],
            ..CompositorConfig::default()
        };

        let Some(bounds) = configured_output_bounds(&config) else {
            panic!("enabled output should produce bounds");
        };
        assert_eq!(bounds.width, 1920.0);
        assert_eq!(bounds.height, 1080.0);
    }

    #[test]
    fn relative_motion_stays_inside_output_bounds() {
        let input_bounds = live_output_bounds([OutputProperties {
            width: 800,
            height: 600,
            refresh_millihz: 60_000,
            scale: 1,
        }])
        .unwrap_or_else(|| panic!("one live output should define bounds"));
        let mut input = DrmInputState { bounds: input_bounds, ..DrmInputState::default() };

        let (x, y) = input.apply_relative_motion(999.0, 999.0);
        assert_eq!(x, 799.0);
        assert_eq!(y, 599.0);
    }
}
