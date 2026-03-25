#[derive(bevy_ecs::system::SystemParam)]
pub(crate) struct FlushProtocolQueueParams<'w> {
    pub(crate) pending_xdg_requests:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PendingXdgRequests>,
    pub(crate) pending_popup_events:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PendingPopupEvents>,
    pub(crate) pending_window_events:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PendingWindowEvents>,
    pub(crate) pending_layer_requests:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PendingLayerRequests>,
    pub(crate) pending_window_controls:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PendingWindowControls>,
    pub(crate) pending_output_events:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PendingOutputEvents>,
    pub(crate) seat_registry: bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::SeatRegistry>,
    pub(crate) clipboard_selection:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::ClipboardSelectionState>,
    pub(crate) drag_and_drop:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::DragAndDropState>,
    pub(crate) primary_selection:
        bevy_ecs::prelude::ResMut<'w, nekoland_ecs::resources::PrimarySelectionState>,
}

pub(crate) fn flush_protocol_queue_system(
    mut protocol_state: bevy_ecs::prelude::ResMut<'_, crate::ProtocolState>,
    mut params: FlushProtocolQueueParams<'_>,
) {
    let mut targets = crate::ProtocolFlushTargets {
        pending_xdg_requests: &mut params.pending_xdg_requests,
        pending_popup_events: &mut params.pending_popup_events,
        pending_window_events: &mut params.pending_window_events,
        pending_layer_requests: &mut params.pending_layer_requests,
        pending_window_controls: &mut params.pending_window_controls,
        pending_output_events: &mut params.pending_output_events,
        seat_registry: &mut params.seat_registry,
        clipboard_selection: &mut params.clipboard_selection,
        drag_and_drop: &mut params.drag_and_drop,
        primary_selection: &mut params.primary_selection,
    };
    protocol_state.flush_into_ecs(&mut targets);
}
