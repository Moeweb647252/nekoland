/// Stacking layout strategy — **not yet implemented**.
///
/// # Extension point
///
/// Stacking layout overlaps windows similarly to traditional desktop WMs
/// (each window floats freely but can be stacked/focused in z-order).
/// Implement by:
/// 1. Querying windows with `WindowState::Floating` or a new `Stacked` state
/// 2. Optionally managing a z-order stack via `RenderList` z_index values
///
/// The `LayoutEngine` trait in `layout/mod.rs` defines the interface.
///
/// # Current behaviour
///
/// The system is present but **not registered in the `ShellPlugin` schedule**.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackingLayout;

/// Stacking layout system — currently a no-op placeholder.
///
/// See [`StackingLayout`] for the extension guide.
pub fn stacking_layout_system() {
    // TODO: implement stacking / z-order window management
    tracing::trace!("stacking layout system tick (not yet implemented)");
}
