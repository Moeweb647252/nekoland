use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::{Local, Res, ResMut, Resource};
use nekoland_ecs::events::{
    ExternalCommandFailed, ExternalCommandLaunched, WindowClosed, WindowCreated, WindowMoved,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{
    ClipboardSnapshot, ConfigSnapshot, PopupSnapshot, PrimarySelectionSnapshot, TreeSnapshot,
    WindowSnapshot,
};
use crate::server::{IpcQueryCache, default_socket_path};
use crate::{IpcCommand, IpcReply, IpcRequest};

const SUBSCRIPTION_IO_TIMEOUT: Duration = Duration::from_millis(100);

pub const SUPPORTED_SUBSCRIPTION_TOPIC_NAMES: &[&str] = &[
    "window",
    "popup",
    "workspace",
    "output",
    "command",
    "config",
    "clipboard",
    "primary-selection",
    "focus",
    "tree",
    "all",
];
pub const KNOWN_SUBSCRIPTION_EVENT_NAMES: &[&str] = &[
    "window_created",
    "window_closed",
    "window_moved",
    "window_geometry_changed",
    "window_state_changed",
    "popup_created",
    "popup_dismissed",
    "popup_geometry_changed",
    "popup_grab_changed",
    "outputs_changed",
    "workspaces_changed",
    "command_launched",
    "command_failed",
    "config_changed",
    "clipboard_changed",
    "primary_selection_changed",
    "focus_changed",
    "tree_changed",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscriptionTopic {
    Window,
    Popup,
    Workspace,
    Output,
    Command,
    Config,
    Clipboard,
    PrimarySelection,
    Focus,
    Tree,
    #[default]
    All,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusChangeSnapshot {
    pub previous_surface: Option<u64>,
    pub focused_surface: Option<u64>,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowStateChangeSnapshot {
    pub surface_id: u64,
    pub previous_state: String,
    pub state: String,
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PopupGrabChangeSnapshot {
    pub surface_id: u64,
    pub parent_surface_id: u64,
    pub previous_grab_active: bool,
    pub previous_grab_serial: Option<u32>,
    pub grab_active: bool,
    pub grab_serial: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IpcSubscription {
    pub topic: SubscriptionTopic,
    pub include_payloads: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcSubscriptionEvent {
    pub topic: SubscriptionTopic,
    pub event: String,
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Default, Resource, PartialEq)]
pub struct PendingSubscriptionEvents {
    pub events: Vec<IpcSubscriptionEvent>,
}

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
    last_clipboard: Option<ClipboardSnapshot>,
    last_primary_selection: Option<PrimarySelectionSnapshot>,
}

pub fn subscribe(subscription: &IpcSubscription) -> io::Result<IpcSubscriptionStream> {
    subscribe_to_path(&default_socket_path(), subscription)
}

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

pub(crate) fn subscription_dispatch_system(
    query_cache: Res<IpcQueryCache>,
    mut window_created: MessageReader<WindowCreated>,
    mut window_closed: MessageReader<WindowClosed>,
    mut window_moved: MessageReader<WindowMoved>,
    mut command_launched: MessageReader<ExternalCommandLaunched>,
    mut command_failed: MessageReader<ExternalCommandFailed>,
    mut pending_events: ResMut<PendingSubscriptionEvents>,
    mut snapshots: Local<SubscriptionSnapshotState>,
) {
    for event in window_created.read() {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_created".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in window_closed.read() {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_closed".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in window_moved.read() {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Window,
            event: "window_moved".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in command_launched.read() {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Command,
            event: "command_launched".to_owned(),
            payload: serde_json::to_value(event).ok(),
        });
    }

    for event in command_failed.read() {
        pending_events.events.push(IpcSubscriptionEvent {
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
        snapshots.initialized = true;
        snapshots.last_tree = Some(query_cache.tree.clone());
        snapshots.last_popups = current_popups;
        snapshots.last_config = Some(query_cache.config.clone());
        snapshots.last_clipboard = Some(query_cache.clipboard.clone());
        snapshots.last_primary_selection = Some(query_cache.primary_selection.clone());
        return;
    }

    for popup in current_popups.values() {
        if snapshots.last_popups.contains_key(&popup.surface_id) {
            continue;
        }

        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Popup,
            event: "popup_created".to_owned(),
            payload: serde_json::to_value(popup).ok(),
        });
    }

    for popup in snapshots.last_popups.values() {
        if current_popups.contains_key(&popup.surface_id) {
            continue;
        }

        pending_events.events.push(IpcSubscriptionEvent {
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
            pending_events.events.push(IpcSubscriptionEvent {
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
            pending_events.events.push(IpcSubscriptionEvent {
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

    let last_tree = snapshots
        .last_tree
        .as_ref()
        .expect("subscription snapshots are initialized immediately before access");
    let previous_windows = last_tree
        .windows
        .iter()
        .cloned()
        .map(|window| (window.surface_id, window))
        .collect::<BTreeMap<_, _>>();

    for window in current_windows.values() {
        let Some(previous) = previous_windows.get(&window.surface_id) else {
            continue;
        };

        if window_geometry_changed(previous, window) {
            pending_events.events.push(IpcSubscriptionEvent {
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
            pending_events.events.push(IpcSubscriptionEvent {
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

    if last_tree.outputs != query_cache.tree.outputs {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Output,
            event: "outputs_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree.outputs).ok(),
        });
    }

    if last_tree.workspaces != query_cache.tree.workspaces {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Workspace,
            event: "workspaces_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree.workspaces).ok(),
        });
    }

    if snapshots.last_config.as_ref() != Some(&query_cache.config) {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Config,
            event: "config_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.config).ok(),
        });
    }

    if snapshots.last_clipboard.as_ref() != Some(&query_cache.clipboard) {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Clipboard,
            event: "clipboard_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.clipboard).ok(),
        });
    }

    if snapshots.last_primary_selection.as_ref() != Some(&query_cache.primary_selection) {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::PrimarySelection,
            event: "primary_selection_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.primary_selection).ok(),
        });
    }

    if last_tree.focused_surface != query_cache.tree.focused_surface {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Focus,
            event: "focus_changed".to_owned(),
            payload: serde_json::to_value(FocusChangeSnapshot {
                previous_surface: last_tree.focused_surface,
                focused_surface: query_cache.tree.focused_surface,
            })
            .ok(),
        });
    }

    if tree_structure_changed(last_tree, &query_cache.tree) {
        pending_events.events.push(IpcSubscriptionEvent {
            topic: SubscriptionTopic::Tree,
            event: "tree_changed".to_owned(),
            payload: serde_json::to_value(&query_cache.tree).ok(),
        });
    }

    snapshots.last_tree = Some(query_cache.tree.clone());
    snapshots.last_popups = current_popups;
    snapshots.last_config = Some(query_cache.config.clone());
    snapshots.last_clipboard = Some(query_cache.clipboard.clone());
    snapshots.last_primary_selection = Some(query_cache.primary_selection.clone());
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

fn tree_structure_changed(previous: &TreeSnapshot, current: &TreeSnapshot) -> bool {
    previous.focused_surface != current.focused_surface
        || previous.outputs != current.outputs
        || previous.workspaces != current.workspaces
        || previous.windows != current.windows
        || previous.popups != current.popups
        || previous.render_order != current.render_order
}
