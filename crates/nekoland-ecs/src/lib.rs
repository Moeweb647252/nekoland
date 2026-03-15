//! Shared ECS-facing data model used across the workspace.
//!
//! This crate intentionally stays close to pure data definitions so backend, protocol, shell, and
//! render crates can communicate through stable component/resource/message types.

pub mod bundles;
pub mod components;
pub mod control;
pub mod events;
pub mod kinds;
pub mod presentation_logic;
pub mod resources;
pub mod selectors;
pub mod views;
pub mod workspace_membership;

pub mod prelude {
    pub use crate::bundles::{OutputBundle, WindowBundle, X11WindowBundle};
    pub use crate::components::*;
    pub use crate::control::*;
    pub use crate::events::*;
    pub use crate::kinds::*;
    pub use crate::presentation_logic::*;
    pub use crate::resources::*;
    pub use crate::selectors::*;
    pub use crate::views::*;
    pub use crate::workspace_membership::*;
}
