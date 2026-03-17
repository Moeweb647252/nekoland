use bevy_ecs::prelude::{Query, Res, ResMut};
use nekoland_ecs::resources::{CursorRenderState, GlobalPointerPosition};
use nekoland_ecs::views::OutputRuntime;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorRenderer;

/// Tracks the cursor against the current output layout so present backends can render it without
/// smuggling a fake surface through the composed desktop scene.
pub fn cursor_render_system(
    pointer: Res<GlobalPointerPosition>,
    outputs: Query<OutputRuntime>,
    mut cursor_state: ResMut<CursorRenderState>,
) {
    let next_output = outputs.iter().find(|output| {
        let left = f64::from(output.placement.x);
        let top = f64::from(output.placement.y);
        let right = left + f64::from(output.properties.width.max(1));
        let bottom = top + f64::from(output.properties.height.max(1));
        pointer.x >= left && pointer.x < right && pointer.y >= top && pointer.y < bottom
    });

    if let Some(output) = next_output {
        cursor_state.visible = true;
        cursor_state.output_id = Some(output.id());
        cursor_state.x = pointer.x - f64::from(output.placement.x);
        cursor_state.y = pointer.y - f64::from(output.placement.y);
    } else {
        cursor_state.visible = false;
        cursor_state.output_id = None;
        cursor_state.x = 0.0;
        cursor_state.y = 0.0;
    }

    tracing::trace!(
        visible = cursor_state.visible,
        output_id = ?cursor_state.output_id,
        x = cursor_state.x,
        y = cursor_state.y,
        "cursor render tick"
    );
}

#[cfg(test)]
mod tests {
    use nekoland_core::prelude::NekolandApp;
    use nekoland_core::schedules::RenderSchedule;
    use nekoland_ecs::bundles::OutputBundle;
    use nekoland_ecs::components::{
        OutputDevice, OutputId, OutputKind, OutputPlacement, OutputProperties,
    };
    use nekoland_ecs::resources::{CursorRenderState, GlobalPointerPosition};

    use super::cursor_render_system;

    #[test]
    fn cursor_render_tracks_output_local_position() {
        let mut app = NekolandApp::new("cursor-render-state-test");
        app.inner_mut()
            .init_resource::<GlobalPointerPosition>()
            .init_resource::<CursorRenderState>()
            .add_systems(RenderSchedule, cursor_render_system);

        let output = app
            .inner_mut()
            .world_mut()
            .spawn(OutputBundle {
                output: OutputDevice {
                    name: "DP-1".to_owned(),
                    kind: OutputKind::Nested,
                    make: "Nekoland".to_owned(),
                    model: "test".to_owned(),
                },
                properties: OutputProperties {
                    width: 1920,
                    height: 1080,
                    refresh_millihz: 60_000,
                    scale: 1,
                },
                placement: OutputPlacement { x: 640, y: 360 },
                ..Default::default()
            })
            .id();

        {
            let mut pointer = app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>();
            pointer.x = 700.0;
            pointer.y = 400.0;
        }

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let cursor = app.inner().world().resource::<CursorRenderState>();
        assert!(cursor.visible);
        assert_eq!(cursor.output_id, app.inner().world().get::<OutputId>(output).copied());
        assert_eq!(cursor.x, 60.0);
        assert_eq!(cursor.y, 40.0);
    }

    #[test]
    fn cursor_render_hides_cursor_outside_outputs() {
        let mut app = NekolandApp::new("cursor-render-hidden-test");
        app.inner_mut()
            .init_resource::<GlobalPointerPosition>()
            .init_resource::<CursorRenderState>()
            .add_systems(RenderSchedule, cursor_render_system);

        {
            let mut pointer = app.inner_mut().world_mut().resource_mut::<GlobalPointerPosition>();
            pointer.x = 4000.0;
            pointer.y = 3000.0;
        }

        app.inner_mut().world_mut().run_schedule(RenderSchedule);

        let cursor = app.inner().world().resource::<CursorRenderState>();
        assert!(!cursor.visible);
        assert_eq!(cursor.output_id, None);
    }
}
