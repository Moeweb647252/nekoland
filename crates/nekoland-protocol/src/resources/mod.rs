pub mod clipboard;
pub mod dnd;
pub mod layer_requests;
pub mod output_presentation;
pub mod pending_events;
pub mod popup_requests;
pub mod window_requests;
pub mod x11_requests;

pub use clipboard::*;
pub use dnd::*;
pub use layer_requests::*;
pub use output_presentation::*;
pub use pending_events::*;
pub use popup_requests::*;
pub use window_requests::*;
pub use x11_requests::*;

use nekoland_ecs::kinds::{BackendEvent, CompositorRequest, ProtocolEvent};

impl ProtocolEvent for WindowLifecycleRequest {}
impl ProtocolEvent for LayerLifecycleRequest {}
impl ProtocolEvent for X11LifecycleRequest {}

impl BackendEvent for OutputEventRecord {}
impl BackendEvent for OutputPresentationEventRecord {}

impl CompositorRequest for PopupServerRequest {}
impl CompositorRequest for WindowServerRequest {}
