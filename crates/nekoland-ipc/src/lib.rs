//! IPC protocol, server runtime, query snapshots, and subscription helpers shared by the
//! compositor and the `nekoland-msg` client.

pub mod commands;
pub mod plugin;
pub mod protocol;
pub mod server;
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
