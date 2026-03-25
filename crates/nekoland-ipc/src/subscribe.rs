//! Client-side subscription handshake plus server-side diffing for high-level IPC topics.
//!
//! The server keeps a small snapshot baseline and emits semantic events only when the cached tree
//! or config state changes in a way subscribers care about.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Local, Res, ResMut};
use bevy_ecs::system::SystemParam;
use nekoland_ecs::events::{
    ExternalCommandFailed, ExternalCommandLaunched, OutputConnected, OutputDisconnected,
    WindowClosed, WindowCreated, WindowMoved,
};
use nekoland_ecs::kinds::SubscriptionEventQueue;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{
    ClipboardSnapshot, ConfigSnapshot, KeyboardLayoutsSnapshot, PopupSnapshot,
    PresentAuditOutputSnapshot, PrimarySelectionSnapshot, TreeSnapshot, WindowSnapshot,
    WorkspaceSnapshot,
};
use crate::server::{IpcQueryCache, default_socket_path};
use crate::{IpcCommand, IpcReply, IpcRequest};

const SUBSCRIPTION_IO_TIMEOUT: Duration = Duration::from_millis(100);

/// Canonical topic names accepted by `subscribe` requests.
pub const SUPPORTED_SUBSCRIPTION_TOPIC_NAMES: &[&str] = &[
    "window",
    "popup",
    "workspace",
    "output",
    "command",
    "config",
    "keyboard-layout",
    "clipboard",
    "primary-selection",
    "present-audit",
    "focus",
    "tree",
    "all",
];
/// Semantic event names the server may emit over subscription streams.
pub const KNOWN_SUBSCRIPTION_EVENT_NAMES: &[&str] = &[
    "window_created",
    "window_closed",
    "window_moved",
    "window_opened_or_changed",
    "windows_changed",
    "window_geometry_changed",
    "window_layouts_changed",
    "window_state_changed",
    "popup_created",
    "popup_dismissed",
    "popup_geometry_changed",
    "popup_grab_changed",
    "output_connected",
    "output_disconnected",
    "outputs_changed",
    "workspaces_changed",
    "workspace_activated",
    "command_launched",
    "command_failed",
    "config_changed",
    "keyboard_layouts_changed",
    "keyboard_layout_switched",
    "clipboard_changed",
    "primary_selection_changed",
    "present_audit_changed",
    "focus_changed",
    "window_focus_changed",
    "tree_changed",
];

/// High-level subscription topic selector used during the IPC handshake.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscriptionTopic {
    Window,
    Popup,
    Workspace,
    Output,
    Command,
    Config,
    KeyboardLayout,
    Clipboard,
    PrimarySelection,
    PresentAudit,
    Focus,
    Tree,
    #[default]
    All,
}

/// Focus-change payload emitted for focus-oriented subscription events.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusChangeSnapshot {
    pub previous_surface: Option<u64>,
    pub focused_surface: Option<u64>,
}

/// Window-geometry delta emitted when a window moves or resizes.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowGeometryChangeSnapshot {
    pub surface_id: u64,
    pub previous_x: i32,
    pub previous_y: i32,
    pub previous_width: u32,
    pub previous_height: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Window-state delta emitted when a window changes state.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowStateChangeSnapshot {
    pub surface_id: u64,
    pub previous_state: String,
    pub state: String,
}

/// Popup-geometry delta emitted when popup placement changes.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupGeometryChangeSnapshot {
    pub surface_id: u64,
    pub parent_surface_id: u64,
    pub previous_x: i32,
    pub previous_y: i32,
    pub previous_width: u32,
    pub previous_height: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Popup-grab delta emitted when popup grab status changes.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupGrabChangeSnapshot {
    pub surface_id: u64,
    pub parent_surface_id: u64,
    pub previous_grab_active: bool,
    pub previous_grab_serial: Option<u32>,
    pub grab_active: bool,
    pub grab_serial: Option<u32>,
}

/// Window-focus delta emitted for focused-window subscription events.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowFocusChangeSnapshot {
    pub previous_surface: Option<u64>,
    pub focused_surface: Option<u64>,
}

/// Workspace-activation payload emitted when the active workspace changes.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceActivatedSnapshot {
    pub previous_workspace: Option<u32>,
    pub workspace: Option<u32>,
    pub output: Option<String>,
}

/// Subscription request payload sent by IPC clients.
#[allow(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcSubscription {
    pub topic: SubscriptionTopic,
    pub include_payloads: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

/// One semantic event emitted on an IPC subscription stream.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcSubscriptionEvent {
    pub topic: SubscriptionTopic,
    pub event: String,
    pub payload: Option<Value>,
}

impl nekoland_ecs::kinds::SubscriptionEvent for IpcSubscriptionEvent {}

/// Queue of subscription events awaiting socket fan-out.
pub type PendingSubscriptionEvents = SubscriptionEventQueue<IpcSubscriptionEvent>;

#[cfg(test)]
mod kind_tests {
    use super::IpcSubscriptionEvent;
    use nekoland_ecs::kinds::SubscriptionEvent;

    fn assert_subscription_event<T: SubscriptionEvent>() {}

    #[test]
    fn ipc_subscription_event_implements_subscription_event_trait() {
        assert_subscription_event::<IpcSubscriptionEvent>();
    }
}

/// Blocking reader over a newline-delimited IPC subscription connection.
#[derive(Debug)]
pub struct IpcSubscriptionStream {
    reader: BufReader<UnixStream>,
}

#[derive(Debug, Default)]
pub(crate) struct SubscriptionSnapshotState {
    initialized: bool,
    last_tree: Option<TreeSnapshot>,
    last_popups: BTreeMap<u64, PopupSnapshot>,
    last_config: Option<ConfigSnapshot>,
    last_keyboard_layouts: Option<KeyboardLayoutsSnapshot>,
    last_clipboard: Option<ClipboardSnapshot>,
    last_primary_selection: Option<PrimarySelectionSnapshot>,
    last_present_audit: Option<Vec<PresentAuditOutputSnapshot>>,
}

#[derive(SystemParam)]
pub(crate) struct SubscriptionDispatchMessages<'w, 's> {
    window_created: MessageReader<'w, 's, WindowCreated>,
    window_closed: MessageReader<'w, 's, WindowClosed>,
    window_moved: MessageReader<'w, 's, WindowMoved>,
    output_connected: MessageReader<'w, 's, OutputConnected>,
    output_disconnected: MessageReader<'w, 's, OutputDisconnected>,
    command_launched: MessageReader<'w, 's, ExternalCommandLaunched>,
    command_failed: MessageReader<'w, 's, ExternalCommandFailed>,
    pending_events: ResMut<'w, PendingSubscriptionEvents>,
}

/// Connects to the default compositor socket and starts a subscription stream.
pub fn subscribe(subscription: &IpcSubscription) -> io::Result<IpcSubscriptionStream> {
    subscribe_to_path(&default_socket_path(), subscription)
}

/// Performs the newline-delimited JSON handshake used by the IPC server before the returned
/// stream starts yielding subscription events.
pub fn subscribe_to_path(
    socket_path: &Path,
    subscription: &IpcSubscription,
) -> io::Result<IpcSubscriptionStream> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(SUBSCRIPTION_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(SUBSCRIPTION_IO_TIMEOUT))?;

    let mut request_bytes = serde_json::to_vec(&IpcRequest {
        correlation_id: 1,
        command: IpcCommand::Subscribe(subscription.clone()),
    })
    .map_err(io::Error::other)?;
    request_bytes.push(b'\n');
    stream.write_all(&request_bytes)?;

    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    let bytes_read = reader.read_line(&mut reply)?;
    if bytes_read == 0 {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "IPC server closed the subscription connection without replying",
        ));
    }

    let reply: IpcReply = serde_json::from_str(reply.trim_end()).map_err(io::Error::other)?;
    if !reply.ok {
        return Err(io::Error::other(format!("IPC subscription rejected: {}", reply.message)));
    }

    Ok(IpcSubscriptionStream { reader })
}

impl IpcSubscriptionStream {
    /// Reads one newline-delimited event frame from the stream.
    pub fn read_event(&mut self) -> io::Result<IpcSubscriptionEvent> {
        let mut line = String::new();
        let bytes_read = self.reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "IPC subscription stream closed before delivering an event",
            ));
        }

        serde_json::from_str(line.trim_end()).map_err(io::Error::other)
    }
}

/// Turns ECS messages plus cached query snapshots into higher-level subscription events.
///
/// Message-based events are emitted immediately, while tree/config/selection changes are derived
/// by diffing the current query cache against the previous snapshot baseline.
pub(crate) fn subscription_dispatch_system(
    query_cache: Res<IpcQueryCache>,
    messages: SubscriptionDispatchMessages<'_, '_>,
    mut snapshots: Local<SubscriptionSnapshotState>,
) {
    let SubscriptionDispatchMessages {
        mut window_created,
        mut window_closed,
        mut window_moved,
        mut output_connected,
        mut output_disconnected,
        mut command_launched,
        mut command_failed,
        mut pending_events,
    } = messages;

    for event in window_created.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_created".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in window_closed.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_closed".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in window_moved.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_moved".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in output_connected.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Output,
            event: "output_connected".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in output_disconnected.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Output,
            event: "output_disconnected".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in command_launched.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Command,
            event: "command_launched".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in command_failed.read() {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Command,
            event: "command_failed".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    let current_popups = query_cache
        .tree
        .popups
        .iter()
        .cloned()
        .map(|popup| (popup.surface_id, popup))
        .collect::<BTreeMap<_, _>>();
    let current_windows = query_cache
        .tree
        .windows
        .iter()
        .cloned()
        .map(|window| (window.surface_id, window))
        .collect::<BTreeMap<_, _>>();

    if !snapshots.initialized {
        // The first tick only seeds the diff baseline. Subscribers start receiving live changes
        // after they connect rather than a synthetic replay of the entire current tree.
        snapshots.initialized = true;
        snapshots.last_tree = Some(query_cache.tree.clone());
        snapshots.last_popups = current_popups;
        snapshots.last_config = Some(query_cache.config.clone());
        snapshots.last_keyboard_layouts = Some(query_cache.keyboard_layouts.clone());
        snapshots.last_clipboard = Some(query_cache.clipboard.clone());
        snapshots.last_primary_selection = Some(query_cache.primary_selection.clone());
        snapshots.last_present_audit = Some(query_cache.present_audit.clone());
        return;
    }

    for popup in current_popups.values() {
        if snapshots.last_popups.contains_key(&popup.surface_id) {
            continue;
        }

        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Popup,
            event: "popup_created".to_owned(),
            payload: serde_json::to_value(popup).ok(),
        });
    }

    for popup in snapshots.last_popups.values() {
        if current_popups.contains_key(&popup.surface_id) {
            continue;
        }

        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Popup,
            event: "popup_dismissed".to_owned(),
            payload: serde_json::to_value(popup).ok(),
        });
    }

    for popup in current_popups.values() {
        let Some(previous) = snapshots.last_popups.get(&popup.surface_id) else {
            continue;
        };

        if popup_geometry_changed(previous, popup) {
            pending_events.push(IpcSubscriptionEvent {
                topic: SubscriptionTopic::Popup,
                event: "popup_geometry_changed".to_owned(),
                payload: serde_json::to_value(PopupGeometryChangeSnapshot {
                    surface_id: popup.surface_id,
                    parent_surface_id: popup.parent_surface_id,
                    previous_x: previous.x,
                    previous_y: previous.y,
                    previous_width: previous.width,
                    previous_height: previous.height,
                    x: popup.x,
                    y: popup.y,
                    width: popup.width,
                    height: popup.height,
                })
                .ok(),
            });
        }

        if popup_grab_changed(previous, popup) {
            pending_events.push(IpcSubscriptionEvent {
                topic: SubscriptionTopic::Popup,
                event: "popup_grab_changed".to_owned(),
                payload: serde_json::to_value(PopupGrabChangeSnapshot {
                    surface_id: popup.surface_id,
                    parent_surface_id: popup.parent_surface_id,
                    previous_grab_active: previous.grab_active,
                    previous_grab_serial: previous.grab_serial,
                    grab_active: popup.grab_active,
                    grab_serial: popup.grab_serial,
                })
                .ok(),
            });
        }
    }

    let Some(last_tree) = snapshots.last_tree.as_ref() else {
        snapshots.last_tree = Some(query_cache.tree.clone());
        snapshots.last_popups = current_popups;
        snapshots.last_config = Some(query_cache.config.clone());
        snapshots.last_keyboard_layouts = Some(query_cache.keyboard_layouts.clone());
        snapshots.last_clipboard = Some(query_cache.clipboard.clone());
        snapshots.last_primary_selection = Some(query_cache.primary_selection.clone());
        snapshots.last_present_audit = Some(query_cache.present_audit.clone());
        return;
    };
    let previous_windows = last_tree
        .windows
        .iter()
        .cloned()
        .map(|window| (window.surface_id, window))
        .collect::<BTreeMap<_, _>>();
    let windows_changed = last_tree.windows != query_cache.tree.windows;
    let window_layouts_changed = if last_tree.windows.len() != query_cache.tree.windows.len() {
        true
    } else {
        current_windows.iter().any(|(surface_id, window)| {
            previous_windows
                .get(surface_id)
                .is_none_or(|previous| window_layout_changed(previous, window))
        })
    };

    for window in current_windows.values() {
        let Some(previous) = previous_windows.get(&window.surface_id) else {
            pending_events.push(IpcSubscriptionEvent {
                topic: SubscriptionTopic::Window,
                event: "window_opened_or_changed".to_owned(),
                payload: serde_json::to_value(window).ok(),
            });
            continue;
        };

        if previous != window {
            pending_events.push(IpcSubscriptionEvent {
                topic: SubscriptionTopic::Window,
                event: "window_opened_or_changed".to_owned(),
                payload: serde_json::to_value(window).ok(),
            });
        }

        if window_geometry_changed(previous, window) {
            pending_events.push(IpcSubscriptionEvent {
                topic: SubscriptionTopic::Window,
                event: "window_geometry_changed".to_owned(),
                payload: serde_json::to_value(WindowGeometryChangeSnapshot {
                    surface_id: window.surface_id,
                    previous_x: previous.x,
                    previous_y: previous.y,
                    previous_width: previous.width,
                    previous_height: previous.height,
                    x: window.x,
                    y: window.y,
                    width: window.width,
                    height: window.height,
                })
                .ok(),
            });
        }

        if previous.state != window.state {
            pending_events.push(IpcSubscriptionEvent {
                topic: SubscriptionTopic::Window,
                event: "window_state_changed".to_owned(),
                payload: serde_json::to_value(WindowStateChangeSnapshot {
                    surface_id: window.surface_id,
                    previous_state: previous.state.clone(),
                    state: window.state.clone(),
                })
                .ok(),
            });
        }
    }

    if windows_changed {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "windows_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree.windows).ok(),
        });
    }

    if window_layouts_changed {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_layouts_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree.windows).ok(),
        });
    }

    if last_tree.outputs != query_cache.tree.outputs {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Output,
            event: "outputs_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree.outputs).ok(),
        });
    }

    if last_tree.workspaces != query_cache.tree.workspaces {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Workspace,
            event: "workspaces_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree.workspaces).ok(),
        });
    }

    let previous_active_workspace = active_workspace(last_tree.workspaces.as_slice());
    let active_workspace = active_workspace(query_cache.tree.workspaces.as_slice());
    if previous_active_workspace != active_workspace {
        let active_output = active_workspace.and_then(|workspace| {
            query_cache
                .tree
                .outputs
                .iter()
                .find(|output| output.current_workspace == Some(workspace.id))
                .map(|output| output.name.clone())
        });
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Workspace,
            event: "workspace_activated".to_owned(),
            payload: serde_json::to_value(WorkspaceActivatedSnapshot {
                previous_workspace: previous_active_workspace.map(|workspace| workspace.id),
                workspace: active_workspace.map(|workspace| workspace.id),
                output: active_output,
            })
            .ok(),
        });
    }

    if snapshots.last_config.as_ref() != Some(&query_cache.config) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Config,
            event: "config_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.config).ok(),
        });
    }

    if snapshots.last_keyboard_layouts.as_ref() != Some(&query_cache.keyboard_layouts) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::KeyboardLayout,
            event: "keyboard_layouts_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.keyboard_layouts).ok(),
        });
    }

    let previous_active_keyboard_layout =
        snapshots.last_keyboard_layouts.as_ref().map(|keyboard_layouts| {
            (keyboard_layouts.active_index, keyboard_layouts.active_name.as_str())
        });
    let active_keyboard_layout = (
        query_cache.keyboard_layouts.active_index,
        query_cache.keyboard_layouts.active_name.as_str(),
    );
    if previous_active_keyboard_layout != Some(active_keyboard_layout) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::KeyboardLayout,
            event: "keyboard_layout_switched".to_owned(),
            payload: serde_json::to_value(&query_cache.keyboard_layouts).ok(),
        });
    }

    if snapshots.last_clipboard.as_ref() != Some(&query_cache.clipboard) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Clipboard,
            event: "clipboard_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.clipboard).ok(),
        });
    }

    if snapshots.last_primary_selection.as_ref() != Some(&query_cache.primary_selection) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::PrimarySelection,
            event: "primary_selection_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.primary_selection).ok(),
        });
    }

    if snapshots.last_present_audit.as_ref() != Some(&query_cache.present_audit) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::PresentAudit,
            event: "present_audit_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.present_audit).ok(),
        });
    }

    if last_tree.focused_surface != query_cache.tree.focused_surface {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Focus,
            event: "focus_changed".to_owned(),
            payload: serde_json::to_value(FocusChangeSnapshot {
                previous_surface: last_tree.focused_surface,
                focused_surface: query_cache.tree.focused_surface,
            })
            .ok(),
        });
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Focus,
            event: "window_focus_changed".to_owned(),
            payload: serde_json::to_value(WindowFocusChangeSnapshot {
                previous_surface: last_tree.focused_surface,
                focused_surface: query_cache.tree.focused_surface,
            })
            .ok(),
        });
    }

    if tree_structure_changed(last_tree, &query_cache.tree) {
        pending_events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Tree,
            event: "tree_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree).ok(),
        });
    }

    snapshots.last_tree = Some(query_cache.tree.clone());
    snapshots.last_popups = current_popups;
    snapshots.last_config = Some(query_cache.config.clone());
    snapshots.last_keyboard_layouts = Some(query_cache.keyboard_layouts.clone());
    snapshots.last_clipboard = Some(query_cache.clipboard.clone());
    snapshots.last_primary_selection = Some(query_cache.primary_selection.clone());
    snapshots.last_present_audit = Some(query_cache.present_audit.clone());
}

fn window_geometry_changed(previous: &WindowSnapshot, current: &WindowSnapshot) -> bool {
    previous.x != current.x
        || previous.y != current.y
        || previous.width != current.width
        || previous.height != current.height
}

fn popup_geometry_changed(previous: &PopupSnapshot, current: &PopupSnapshot) -> bool {
    previous.x != current.x
        || previous.y != current.y
        || previous.width != current.width
        || previous.height != current.height
}

fn popup_grab_changed(previous: &PopupSnapshot, current: &PopupSnapshot) -> bool {
    previous.grab_active != current.grab_active || previous.grab_serial != current.grab_serial
}

fn window_layout_changed(previous: &WindowSnapshot, current: &WindowSnapshot) -> bool {
    previous.layout != current.layout
        || previous.role != current.role
        || previous.scene_x != current.scene_x
        || previous.scene_y != current.scene_y
        || previous.width != current.width
        || previous.height != current.height
        || previous.workspace != current.workspace
        || previous.output != current.output
        || previous.render_index != current.render_index
}

fn active_workspace(workspaces: &[WorkspaceSnapshot]) -> Option<&WorkspaceSnapshot> {
    workspaces.iter().find(|workspace| workspace.active)
}

fn tree_structure_changed(previous: &TreeSnapshot, current: &TreeSnapshot) -> bool {
    previous.focused_surface != current.focused_surface
        || previous.outputs != current.outputs
        || previous.workspaces != current.workspaces
        || previous.windows != current.windows
        || previous.popups != current.popups
        || previous.render_order != current.render_order
}

#[cfg(test)]
mod tests {
    use bevy_ecs::message::Messages;
    use bevy_ecs::prelude::World;
    use bevy_ecs::schedule::Schedule;

    use nekoland_ecs::events::{
        ExternalCommandFailed, ExternalCommandLaunched, OutputConnected, OutputDisconnected,
        WindowClosed, WindowCreated, WindowMoved,
    };

    use crate::commands::{PresentAuditElementSnapshot, PresentAuditOutputSnapshot};

    use super::{PendingSubscriptionEvents, SubscriptionTopic, subscription_dispatch_system};

    #[test]
    fn subscription_dispatch_emits_present_audit_changed_when_audit_snapshot_changes() {
        let mut world = World::default();
        world.insert_resource(crate::server::IpcQueryCache::default());
        world.insert_resource(PendingSubscriptionEvents::default());
        world.insert_resource(Messages::<WindowCreated>::default());
        world.insert_resource(Messages::<WindowClosed>::default());
        world.insert_resource(Messages::<WindowMoved>::default());
        world.insert_resource(Messages::<OutputConnected>::default());
        world.insert_resource(Messages::<OutputDisconnected>::default());
        world.insert_resource(Messages::<ExternalCommandLaunched>::default());
        world.insert_resource(Messages::<ExternalCommandFailed>::default());

        let mut schedule = Schedule::default();
        schedule.add_systems(subscription_dispatch_system);

        schedule.run(&mut world);
        assert!(
            world.resource::<PendingSubscriptionEvents>().is_empty(),
            "initial subscription tick should only seed baselines"
        );

        world.resource_mut::<crate::server::IpcQueryCache>().present_audit =
            vec![PresentAuditOutputSnapshot {
                output_name: "Virtual-1".to_owned(),
                frame: 4,
                uptime_millis: 44,
                elements: vec![PresentAuditElementSnapshot {
                    surface_id: 42,
                    kind: "window".to_owned(),
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                    z_index: 0,
                    opacity: 1.0,
                }],
            }];

        schedule.run(&mut world);

        let events = world.resource::<PendingSubscriptionEvents>().as_slice().to_vec();
        assert_eq!(events.len(), 1, "expected one present-audit subscription event");
        assert_eq!(events[0].topic, SubscriptionTopic::PresentAudit);
        assert_eq!(events[0].event, "present_audit_changed");
        assert!(events[0].payload.is_some());
    }
}
