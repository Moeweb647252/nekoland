use std::collections::BTreeSet;

use bevy_ecs::prelude::{Query, ResMut, With};
use nekoland_ecs::components::{
    BufferState, LayerLevel, LayerShellSurface, LayoutSlot, WindowAnimation, WindowState,
    WlSurfaceHandle, Workspace, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{RenderElement, RenderList};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameComposer;

pub fn compose_frame_system(
    layers: Query<
        (&WlSurfaceHandle, &WindowAnimation, &LayerShellSurface, &BufferState),
        With<LayerShellSurface>,
    >,
    windows: Query<
        (&WlSurfaceHandle, &WindowAnimation, &WindowState, &LayoutSlot),
        With<XdgWindow>,
    >,
    popups: Query<(&WlSurfaceHandle, &WindowAnimation, &XdgPopup), With<XdgPopup>>,
    workspaces: Query<&Workspace>,
    mut render_list: ResMut<RenderList>,
) {
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map(|workspace| workspace.id.0)
        .or_else(|| {
            workspaces.iter().min_by_key(|workspace| workspace.id).map(|workspace| workspace.id.0)
        });
    let active_window_surfaces = windows
        .iter()
        .filter(|(_, _, state, layout_slot)| {
            **state != WindowState::Hidden
                && active_workspace.is_none_or(|workspace| layout_slot.workspace == workspace)
        })
        .map(|(surface, _, _, _)| surface.id)
        .collect::<BTreeSet<_>>();
    let mut elements = layers
        .iter()
        .filter(|(_, _, _, buffer)| buffer.attached)
        .filter(|(_, _, layer_surface, _)| {
            matches!(layer_surface.layer, LayerLevel::Background | LayerLevel::Bottom)
        })
        .map(|(surface, animation, _, _)| (surface, animation))
        .chain(
            windows
                .iter()
                .filter(|(surface, _, _, _)| active_window_surfaces.contains(&surface.id))
                .map(|(surface, animation, _, _)| (surface, animation)),
        )
        .chain(
            popups
                .iter()
                .filter(|(_, _, popup)| active_window_surfaces.contains(&popup.parent_surface))
                .map(|(surface, animation, _)| (surface, animation)),
        )
        .chain(
            layers
                .iter()
                .filter(|(_, _, layer_surface, buffer)| {
                    buffer.attached
                        && matches!(layer_surface.layer, LayerLevel::Top | LayerLevel::Overlay)
                })
                .map(|(surface, animation, _, _)| (surface, animation)),
        )
        .enumerate()
        .map(|(z_index, (surface, animation))| RenderElement {
            surface_id: surface.id,
            z_index: z_index as i32,
            opacity: if animation.progress == 0.0 { 1.0 } else { animation.progress },
        })
        .collect::<Vec<_>>();

    elements.sort_by_key(|element| element.z_index);
    render_list.elements = elements;

    tracing::trace!(elements = render_list.elements.len(), "frame composition tick");
}
