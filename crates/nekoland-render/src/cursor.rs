use bevy_ecs::prelude::{Res, ResMut};
use nekoland_ecs::resources::{GlobalPointerPosition, RenderElement, RenderList};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorRenderer;

pub fn cursor_render_system(
    pointer: Res<GlobalPointerPosition>,
    mut render_list: ResMut<RenderList>,
) {
    render_list.elements.push(RenderElement { surface_id: 0, z_index: i32::MAX, opacity: 1.0 });

    tracing::trace!(x = pointer.x, y = pointer.y, "cursor render tick");
}
