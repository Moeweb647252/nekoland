use std::cell::RefCell;
use std::rc::Rc;

use bevy_app::App;
use nekoland_core::calloop::CalloopSourceRegistry;
use nekoland_core::error::NekolandError;
use nekoland_ecs::resources::{
    BackendInputAction, BackendInputEvent, PendingBackendInputEvents, PendingProtocolInputEvents,
};
use smithay::backend::session::Event as SessionEvent;
use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;

use crate::traits::BackendDescriptor;

use super::device::SharedDrmState;
use super::gbm::SharedGbmState;
use super::input::SharedDrmInputState;
use super::surface::DrmRenderState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmSessionStatus {
    Uninitialized,
    Ready,
    Failed(String),
}

#[derive(Debug)]
pub struct DrmSessionState {
    pub status: DrmSessionStatus,
    pub session: Option<LibSeatSession>,
    pub seat_name: String,
    pub active: bool,
    pub was_active: Option<bool>,
}

impl Default for DrmSessionState {
    fn default() -> Self {
        Self {
            status: DrmSessionStatus::Uninitialized,
            session: None,
            seat_name: default_seat_name(),
            active: false,
            was_active: None,
        }
    }
}

pub type SharedDrmSessionState = Rc<RefCell<DrmSessionState>>;

pub(crate) fn install_drm_session_source(app: &mut App, session_state: SharedDrmSessionState) {
    if app.world().get_non_send_resource::<CalloopSourceRegistry>().is_none() {
        app.insert_non_send_resource(CalloopSourceRegistry::default());
    }

    let Some(mut registry) = app.world_mut().get_non_send_resource_mut::<CalloopSourceRegistry>()
    else {
        let message = "calloop registry unavailable during drm session installation".to_owned();
        tracing::error!(error = %message);
        let mut state = session_state.borrow_mut();
        state.status = DrmSessionStatus::Failed(message);
        state.session = None;
        state.active = false;
        state.was_active = Some(false);
        return;
    };

    registry.push(move |handle| {
        let (session, notifier) = match LibSeatSession::new() {
            Ok(pair) => pair,
            Err(error) => {
                let message = format!("failed to create libseat session: {error}");
                let mut state = session_state.borrow_mut();
                state.status = DrmSessionStatus::Failed(message.clone());
                state.session = None;
                state.active = false;
                state.was_active = Some(false);
                return Err(NekolandError::Runtime(message));
            }
        };

        let seat_name = session.seat();
        let active = session.is_active();
        {
            let mut state = session_state.borrow_mut();
            state.status = DrmSessionStatus::Ready;
            state.session = Some(session);
            state.seat_name = seat_name;
            state.active = active;
            state.was_active = Some(active);
        }

        let session_state_for_events = session_state.clone();
        handle
            .insert_source(notifier, move |event, _, _| {
                let mut state = session_state_for_events.borrow_mut();
                match event {
                    SessionEvent::ActivateSession => state.active = true,
                    SessionEvent::PauseSession => state.active = false,
                }
            })
            .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;

        Ok(())
    });
}

pub(crate) fn extract_session(
    descriptor: &mut BackendDescriptor,
    session_state: &SharedDrmSessionState,
    drm_state: &SharedDrmState,
    gbm_state: &SharedGbmState,
    input_state: &SharedDrmInputState,
    render_state: &mut DrmRenderState,
    pending_backend_inputs: &mut PendingBackendInputEvents,
    pending_protocol_inputs: &mut PendingProtocolInputEvents,
) {
    let mut session_state = session_state.borrow_mut();
    match &session_state.status {
        DrmSessionStatus::Uninitialized => {
            descriptor.description = "drm backend initializing tty session".to_owned();
            return;
        }
        DrmSessionStatus::Failed(error) => {
            descriptor.description = format!("drm backend unavailable: {error}");
            return;
        }
        DrmSessionStatus::Ready => {}
    }

    let active = session_state.active;
    let seat_name = session_state.seat_name.clone();
    let transition = take_activity_transition(&mut session_state);
    drop(session_state);

    descriptor.description = if active {
        format!("drm backend on seat {seat_name}")
    } else {
        format!("drm backend on seat {seat_name} (tty inactive)")
    };

    if transition == Some(false) {
        *drm_state.borrow_mut() = None;
        *gbm_state.borrow_mut() = None;
        input_state.borrow_mut().pending_input_events.clear();
        render_state.renderer = None;
        render_state.surfaces.clear();
    }

    if let Some(focused) = transition {
        let focus_event = BackendInputEvent {
            device: "tty".to_owned(),
            action: BackendInputAction::FocusChanged { focused },
        };
        pending_backend_inputs.push(focus_event.clone());
        pending_protocol_inputs.push(focus_event);
    }
}

fn take_activity_transition(state: &mut DrmSessionState) -> Option<bool> {
    if state.status != DrmSessionStatus::Ready {
        return None;
    }

    if state.was_active == Some(state.active) {
        return None;
    }

    state.was_active = Some(state.active);
    Some(state.active)
}

fn default_seat_name() -> String {
    std::env::var("NEKOLAND_SEAT").unwrap_or_else(|_| "seat0".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{DrmSessionState, DrmSessionStatus, take_activity_transition};

    #[test]
    fn reports_focus_transition_when_session_reactivates() {
        let mut state = DrmSessionState {
            status: DrmSessionStatus::Ready,
            active: true,
            was_active: Some(false),
            ..DrmSessionState::default()
        };

        assert_eq!(take_activity_transition(&mut state), Some(true));
        assert_eq!(state.was_active, Some(true));
        assert_eq!(take_activity_transition(&mut state), None);
    }

    #[test]
    fn ignores_activity_transition_when_session_not_ready() {
        let mut state = DrmSessionState::default();
        state.active = true;

        assert_eq!(take_activity_transition(&mut state), None);
        assert_eq!(state.was_active, None);
    }
}
