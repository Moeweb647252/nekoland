#![warn(missing_docs)]

//! IPC protocol, server runtime, query snapshots, and subscription helpers shared by the
//! compositor and the `nekoland-msg` client.

/// Public command/query/action model used by the compositor and CLI.
pub mod commands;
/// Main-world plugin entrypoint that wires IPC request handling and query-cache refresh.
pub mod plugin;
/// Wire-level IPC request and reply envelopes.
pub mod protocol;
/// Unix-socket server runtime and query cache.
pub mod server;
/// Subscription topics, diffing helpers, and client-side streaming helpers.
pub mod subscribe;

pub use plugin::IpcPlugin;
pub use protocol::{IpcCommand, IpcReply, IpcRequest};
pub use server::{
    IpcQueryCache, IpcServerState, default_socket_path, send_request, send_request_to_path,
};
pub use subscribe::{
    FocusChangeSnapshot, IpcSubscription, IpcSubscriptionEvent, IpcSubscriptionStream,
    KNOWN_SUBSCRIPTION_EVENT_NAMES, PendingSubscriptionEvents, PopupGeometryChangeSnapshot,
    PopupGrabChangeSnapshot, SUPPORTED_SUBSCRIPTION_TOPIC_NAMES, SubscriptionTopic,
    WindowFocusChangeSnapshot, WindowGeometryChangeSnapshot, WindowStateChangeSnapshot,
    WorkspaceActivatedSnapshot, subscribe, subscribe_to_path,
};
