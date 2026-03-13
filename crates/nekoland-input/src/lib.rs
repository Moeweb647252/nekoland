//! Input decoding, gesture/keybinding dispatch, and seat bookkeeping.

pub mod gestures;
pub mod keybindings;
pub mod keyboard;
pub mod plugin;
pub mod pointer;
pub mod seat_manager;
pub mod touch;

pub use plugin::InputPlugin;
