pub mod floating;
pub mod fullscreen;
pub mod stacking;
pub mod tiling;

use bevy_ecs::prelude::Query;
use nekoland_ecs::components::{SurfaceGeometry, WindowState, XdgWindow};
use nekoland_ecs::resources::WorkArea;

/// Trait that all layout strategies implement.
///
/// Each strategy receives the full window query and the current work area and
/// is responsible for updating `SurfaceGeometry` for any windows whose
/// [`WindowState`] it manages. Strategies that are not yet implemented should
/// leave the query untouched and return immediately.
///
/// # Future extensibility
///
/// To add a new layout engine, implement this trait and register it as a
/// Bevy resource.  The [`ShellPlugin`](crate::plugin::ShellPlugin) can then
/// query for `Res<dyn LayoutEngine>` or simply dispatch a concrete system.
/// Keeping strategies behind this boundary ensures that adding tiling or
/// stacking later requires only a new `impl LayoutEngine` and a one-line
/// change to the plugin.
pub trait LayoutEngine: Send + Sync + 'static {
    /// Human-readable name used in trace spans.
    fn name(&self) -> &'static str;

    /// Apply geometry updates for the current frame.
    ///
    /// Implementors should only mutate windows whose state matches their
    /// strategy.  Overlapping state assignments (e.g. a window set to
    /// `Fullscreen`) are handled by dedicated systems that run after the
    /// primary layout pass.
    fn apply(
        &self,
        windows: &mut Query<
            '_,
            '_,
            (&mut SurfaceGeometry, &WindowState),
            bevy_ecs::prelude::With<XdgWindow>,
        >,
        work_area: &WorkArea,
    );
}
