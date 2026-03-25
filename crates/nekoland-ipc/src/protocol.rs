//! Wire-level request and reply envelopes exchanged over the IPC socket.

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{
    ActionCommand, OutputCommand, PopupCommand, QueryCommand, TilingCommand, WindowCommand,
    WorkspaceCommand,
};
use crate::subscribe::IpcSubscription;

/// Top-level IPC command envelope exchanged between clients and the compositor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IpcCommand {
    Action(ActionCommand),
    Tiling(TilingCommand),
    Window(WindowCommand),
    Popup(PopupCommand),
    Subscribe(IpcSubscription),
    Workspace(WorkspaceCommand),
    Output(OutputCommand),
    Query(QueryCommand),
    Raw(String),
}

/// One IPC request frame sent by a client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcRequest {
    pub correlation_id: u64,
    pub command: IpcCommand,
}

/// One IPC reply frame returned by the compositor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IpcReply {
    pub ok: bool,
    pub message: String,
    pub payload: Option<Value>,
}
