//! Input decoding, gesture/keybinding dispatch, and seat bookkeeping.
#![warn(missing_docs)]

/// Gesture recognition built on top of normalized touch and pointer state.
pub mod gestures;
/// Keybinding compilation and action dispatch driven by the active config.
pub mod keybindings;
/// Keyboard-event decoding and modifier / pressed-key state tracking.
pub mod keyboard;
/// Plugin entrypoint that wires the input schedule together.
pub mod plugin;
/// Pointer-motion decoding, viewport panning, and focused-output tracking.
pub mod pointer;
/// Seat-facing glue that finalizes normalized input state for shell consumers.
pub mod seat_manager;
/// Touch-event decoding used by gesture recognition and future shell policy.
pub mod touch;

pub use plugin::InputPlugin;
