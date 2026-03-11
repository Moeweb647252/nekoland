/// Tiling layout strategy — **not yet implemented**.
///
/// # Extension point
///
/// Implement tiling by:
/// 1. Querying windows with `WindowState::Tiled`
/// 2. Dividing the `WorkArea` into columns or rows based on window count
/// 3. Assigning each window a `SurfaceGeometry` slice
///
/// The `LayoutEngine` trait in `layout/mod.rs` defines the interface any
/// future strategy should implement.
///
/// # Current behaviour
///
/// The system is present in the module but **not registered in the
/// `ShellPlugin` schedule**.  Windows are kept in floating layout until
/// tiling is implemented.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TilingLayout;

/// Tiling layout system — currently a no-op placeholder.
///
/// See [`TilingLayout`] for the extension guide.
pub fn tiling_layout_system() {
    // TODO: implement column/row tiling layout
    tracing::trace!("tiling layout system tick (not yet implemented)");
}
