use std::collections::{BTreeSet, HashMap};
use std::io::{ErrorKind, Read};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub(crate) struct PersistedSelectionData {
    pub(crate) mime_data: Arc<HashMap<String, Vec<u8>>>,
}

#[derive(Debug)]
struct PendingSelectionCapture {
    mime_type: String,
    reader: UnixStream,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct SelectionCaptureRequest {
    generation: u64,
    mime_types: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct SelectionCaptureState {
    generation: u64,
    installed_generation: Option<u64>,
    pending_request: Option<SelectionCaptureRequest>,
    active_captures: Vec<PendingSelectionCapture>,
    captured_mime_data: HashMap<String, Vec<u8>>,
}

impl SelectionCaptureState {
    fn note_selection_change(&mut self, mime_types: Vec<String>) {
        self.generation = self.generation.saturating_add(1);
        self.installed_generation = None;
        self.pending_request =
            Some(SelectionCaptureRequest { generation: self.generation, mime_types });
        self.active_captures.clear();
        self.captured_mime_data.clear();
    }
}

#[derive(Debug, Default)]
pub(crate) struct SelectionPersistenceState {
    pub(crate) clipboard: SelectionCaptureState,
    pub(crate) primary: SelectionCaptureState,
}

impl SelectionPersistenceState {
    pub(crate) fn note_selection_change(
        &mut self,
        target: super::SelectionTarget,
        mime_types: Vec<String>,
    ) {
        match target {
            super::SelectionTarget::Clipboard => self.clipboard.note_selection_change(mime_types),
            super::SelectionTarget::Primary => self.primary.note_selection_change(mime_types),
        }
    }
}

#[derive(Debug)]
enum SelectionCapturePoll {
    Pending(PendingSelectionCapture),
    Complete { mime_type: String, bytes: Vec<u8> },
    Drop,
}

fn poll_selection_capture(mut capture: PendingSelectionCapture) -> SelectionCapturePoll {
    loop {
        let mut buffer = [0_u8; 4096];
        match capture.reader.read(&mut buffer) {
            Ok(0) => {
                return SelectionCapturePoll::Complete {
                    mime_type: capture.mime_type,
                    bytes: capture.bytes,
                };
            }
            Ok(read) => {
                capture.bytes.extend_from_slice(&buffer[..read]);
                if capture.bytes.len() > super::MAX_PERSISTED_SELECTION_BYTES {
                    tracing::warn!(
                        %capture.mime_type,
                        limit = super::MAX_PERSISTED_SELECTION_BYTES,
                        "dropping oversized persisted selection payload"
                    );
                    return SelectionCapturePoll::Drop;
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                return SelectionCapturePoll::Pending(capture);
            }
            Err(error) => {
                tracing::warn!(
                    %capture.mime_type,
                    %error,
                    "failed while reading persisted selection payload"
                );
                return SelectionCapturePoll::Drop;
            }
        }
    }
}

fn selection_target_name(target: super::SelectionTarget) -> &'static str {
    match target {
        super::SelectionTarget::Clipboard => "clipboard",
        super::SelectionTarget::Primary => "primary-selection",
    }
}

pub(crate) fn process_selection_persistence_system(
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    server.process_selection_persistence();
}

impl super::server::SmithayProtocolRuntime {
    pub(crate) fn process_selection_persistence(&mut self) {
        self.process_selection_capture_requests(super::SelectionTarget::Clipboard);
        self.process_selection_capture_requests(super::SelectionTarget::Primary);
        self.poll_selection_captures(super::SelectionTarget::Clipboard);
        self.poll_selection_captures(super::SelectionTarget::Primary);
    }

    fn process_selection_capture_requests(&mut self, target: super::SelectionTarget) {
        let Some(request) = self.capture_state_mut(target).pending_request.take() else {
            return;
        };

        let capture_state = self.capture_state_mut(target);
        capture_state.active_captures.clear();
        capture_state.captured_mime_data.clear();
        capture_state.installed_generation = None;

        if request.mime_types.is_empty() {
            self.clear_persisted_selection(target);
            return;
        }

        let mut scheduled = Vec::new();
        for mime_type in request.mime_types.into_iter().collect::<BTreeSet<_>>() {
            let Ok((reader, writer)) = UnixStream::pair() else {
                tracing::warn!(
                    selection = selection_target_name(target),
                    %mime_type,
                    "failed to allocate selection persistence pipe"
                );
                continue;
            };
            if let Err(error) = reader.set_nonblocking(true) {
                tracing::warn!(
                    selection = selection_target_name(target),
                    %mime_type,
                    %error,
                    "failed to configure selection persistence reader"
                );
                continue;
            }

            let request_failed =
                match target {
                    super::SelectionTarget::Clipboard => {
                        super::request_data_device_client_selection::<
                            super::server::ProtocolRuntimeState,
                        >(&self.state.seat, mime_type.clone(), writer.into())
                        .map_err(|error| error.to_string())
                    }
                    super::SelectionTarget::Primary => super::request_primary_client_selection::<
                        super::server::ProtocolRuntimeState,
                    >(
                        &self.state.seat,
                        mime_type.clone(),
                        writer.into(),
                    )
                    .map_err(|error| error.to_string()),
                };

            if let Err(error) = request_failed {
                tracing::debug!(
                    selection = selection_target_name(target),
                    %mime_type,
                    %error,
                    "selection persistence request was not accepted"
                );
                continue;
            }

            scheduled.push(PendingSelectionCapture { mime_type, reader, bytes: Vec::new() });
        }

        self.capture_state_mut(target).active_captures = scheduled;
        self.capture_state_mut(target).generation = request.generation;

        if let Err(error) = self.display.flush_clients() {
            super::server::remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after scheduling selection persistence",
            );
        }
    }

    fn poll_selection_captures(&mut self, target: super::SelectionTarget) {
        let generation = self.capture_state_mut(target).generation;
        let captures = std::mem::take(&mut self.capture_state_mut(target).active_captures);
        let mut pending = Vec::new();

        for capture in captures {
            match poll_selection_capture(capture) {
                SelectionCapturePoll::Pending(capture) => pending.push(capture),
                SelectionCapturePoll::Complete { mime_type, bytes } => {
                    self.capture_state_mut(target).captured_mime_data.insert(mime_type, bytes);
                }
                SelectionCapturePoll::Drop => {}
            }
        }

        self.capture_state_mut(target).active_captures = pending;
        let should_install = {
            let state = self.capture_state_mut(target);
            state.active_captures.is_empty()
                && !state.captured_mime_data.is_empty()
                && state.installed_generation != Some(generation)
        };

        if !should_install {
            return;
        }

        let persisted = PersistedSelectionData {
            mime_data: Arc::new(self.capture_state_mut(target).captured_mime_data.clone()),
        };
        self.install_persisted_selection(target, persisted);
        self.capture_state_mut(target).installed_generation = Some(generation);
    }

    fn install_persisted_selection(
        &mut self,
        target: super::SelectionTarget,
        persisted: PersistedSelectionData,
    ) {
        let mime_types = persisted.mime_data.keys().cloned().collect::<Vec<_>>();
        match target {
            super::SelectionTarget::Clipboard => {
                super::set_data_device_selection::<super::server::ProtocolRuntimeState>(
                    &self.display.handle(),
                    &self.state.seat,
                    mime_types.clone(),
                    persisted,
                )
            }
            super::SelectionTarget::Primary => {
                super::set_primary_selection::<super::server::ProtocolRuntimeState>(
                    &self.display.handle(),
                    &self.state.seat,
                    mime_types.clone(),
                    persisted,
                )
            }
        }
        match target {
            super::SelectionTarget::Clipboard => {
                self.state.event_queue.push_back(
                    super::ProtocolEvent::ClipboardSelectionPersisted {
                        persisted_mime_types: mime_types,
                    },
                );
            }
            super::SelectionTarget::Primary => {
                self.state.event_queue.push_back(super::ProtocolEvent::PrimarySelectionPersisted {
                    persisted_mime_types: mime_types,
                });
            }
        }

        if let Err(error) = self.display.flush_clients() {
            super::server::remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after installing persisted selection",
            );
        }
    }

    fn clear_persisted_selection(&mut self, target: super::SelectionTarget) {
        match target {
            super::SelectionTarget::Clipboard => {
                super::clear_data_device_selection::<super::server::ProtocolRuntimeState>(
                    &self.display.handle(),
                    &self.state.seat,
                );
            }
            super::SelectionTarget::Primary => {
                super::clear_primary_selection::<super::server::ProtocolRuntimeState>(
                    &self.display.handle(),
                    &self.state.seat,
                );
            }
        }

        if let Err(error) = self.display.flush_clients() {
            super::server::remember_protocol_error(
                &mut self.last_dispatch_error,
                error,
                "failed to flush Wayland clients after clearing persisted selection",
            );
        }
    }

    fn capture_state_mut(&mut self, target: super::SelectionTarget) -> &mut SelectionCaptureState {
        match target {
            super::SelectionTarget::Clipboard => &mut self.state.selection_persistence.clipboard,
            super::SelectionTarget::Primary => &mut self.state.selection_persistence.primary,
        }
    }
}

impl super::SelectionHandler for super::server::ProtocolRuntimeState {
    type SelectionUserData = PersistedSelectionData;

    fn new_selection(
        &mut self,
        ty: super::SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        seat: smithay::input::Seat<Self>,
    ) {
        let seat_name = seat.name().to_owned();
        let mime_types = source.map(|source| source.mime_types()).unwrap_or_default();
        self.selection_persistence.note_selection_change(ty, mime_types.clone());

        match ty {
            super::SelectionTarget::Clipboard => {
                self.event_queue.push_back(super::ProtocolEvent::ClipboardSelectionChanged {
                    seat_name,
                    mime_types,
                });
            }
            super::SelectionTarget::Primary => {
                self.event_queue.push_back(super::ProtocolEvent::PrimarySelectionChanged {
                    seat_name,
                    mime_types,
                });
            }
        }
    }

    fn send_selection(
        &mut self,
        _ty: super::SelectionTarget,
        mime_type: String,
        fd: std::os::unix::io::OwnedFd,
        _seat: smithay::input::Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        let Some(bytes) = user_data.mime_data.get(&mime_type) else {
            tracing::warn!(%mime_type, "requested persisted selection mime type is unavailable");
            return;
        };

        let mut file = std::fs::File::from(fd);
        if let Err(error) = std::io::Write::write_all(&mut file, bytes) {
            tracing::warn!(%mime_type, %error, "failed to write persisted selection payload");
        }
    }
}

impl super::PrimarySelectionHandler for super::server::ProtocolRuntimeState {
    fn primary_selection_state(&self) -> &super::SmithayPrimarySelectionState {
        &self._primary_selection_state
    }
}

impl super::ClientDndGrabHandler for super::server::ProtocolRuntimeState {
    fn started(
        &mut self,
        source: Option<smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource>,
        icon: Option<super::WlSurface>,
        seat: smithay::input::Seat<Self>,
    ) {
        let source_surface_id = seat
            .get_pointer()
            .and_then(|pointer| pointer.grab_start_data())
            .and_then(|start_data| start_data.focus.map(|(surface, _)| self.surface_id(&surface)));
        let icon_surface_id = icon.as_ref().map(|surface| self.surface_id(surface));
        let mime_types = source
            .as_ref()
            .and_then(|source| {
                super::with_source_metadata(source, |metadata| metadata.mime_types.clone()).ok()
            })
            .unwrap_or_default();

        self.queue_event(super::ProtocolEvent::DragStarted {
            seat_name: seat.name().to_owned(),
            source_surface_id,
            icon_surface_id,
            mime_types,
        });
    }

    fn dropped(
        &mut self,
        target: Option<super::WlSurface>,
        validated: bool,
        seat: smithay::input::Seat<Self>,
    ) {
        let target_surface_id = target.as_ref().map(|surface| self.surface_id(surface));
        self.queue_event(super::ProtocolEvent::DragDropped {
            seat_name: seat.name().to_owned(),
            target_surface_id,
            validated,
        });
    }
}

impl super::ServerDndGrabHandler for super::server::ProtocolRuntimeState {
    fn accept(&mut self, mime_type: Option<String>, seat: smithay::input::Seat<Self>) {
        self.queue_event(super::ProtocolEvent::DragAccepted {
            seat_name: seat.name().to_owned(),
            mime_type,
        });
    }

    fn action(
        &mut self,
        action: smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction,
        seat: smithay::input::Seat<Self>,
    ) {
        self.queue_event(super::ProtocolEvent::DragActionSelected {
            seat_name: seat.name().to_owned(),
            action: format!("{action:?}"),
        });
    }
}

impl super::DataDeviceHandler for super::server::ProtocolRuntimeState {
    fn data_device_state(&self) -> &super::SmithayDataDeviceState {
        &self.data_device_state
    }
}
