use std::collections::HashMap;

use bevy_ecs::prelude::{Local, Query, Res, ResMut};
use nekoland_ecs::components::{
    LayerShellSurface, OutputDevice, SurfaceGeometry, WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::resources::{
    CompositorClock, CompositorConfig, GlobalPointerPosition, PendingOutputPresentationEvents,
    VirtualOutputCaptureState, VirtualOutputElement, VirtualOutputElementKind, VirtualOutputFrame,
};
use smithay::utils::{Clock, Monotonic};

use crate::plugin::{OutputPresentationRuntime, emit_present_completion_events};
use crate::traits::{BackendKind, SelectedBackend};

const DEFAULT_CURSOR_SIZE: u32 = 24;

pub(crate) fn virtual_backend_system(mut selected_backend: ResMut<SelectedBackend>) {
    if selected_backend.kind != BackendKind::Virtual {
        return;
    }

    selected_backend.description = "offscreen virtual output backend".to_owned();
}

pub(crate) fn virtual_present_completion_system(
    selected_backend: Res<SelectedBackend>,
    outputs: Query<(&OutputDevice, &nekoland_ecs::components::OutputProperties)>,
    mut pending_presentation_events: ResMut<PendingOutputPresentationEvents>,
    mut presentation_runtime: Local<OutputPresentationRuntime>,
    mut monotonic_clock: Local<Option<Clock<Monotonic>>>,
) {
    emit_present_completion_events(
        BackendKind::Virtual,
        &selected_backend,
        &outputs,
        &mut pending_presentation_events,
        &mut presentation_runtime,
        &mut monotonic_clock,
    );
}

pub(crate) fn virtual_output_capture_system(
    selected_backend: Res<SelectedBackend>,
    clock: Res<CompositorClock>,
    config: Option<Res<CompositorConfig>>,
    outputs: Query<(&OutputDevice, &nekoland_ecs::components::OutputProperties)>,
    pointer: Option<Res<GlobalPointerPosition>>,
    render_list: Res<nekoland_ecs::resources::RenderList>,
    surfaces: Query<(
        &WlSurfaceHandle,
        &SurfaceGeometry,
        Option<&XdgWindow>,
        Option<&XdgPopup>,
        Option<&LayerShellSurface>,
    )>,
    mut capture_state: ResMut<VirtualOutputCaptureState>,
) {
    if selected_backend.kind != BackendKind::Virtual {
        return;
    }

    let Some((output, properties)) = outputs.iter().next() else {
        return;
    };

    let geometry_by_surface = surfaces
        .iter()
        .map(|(surface, geometry, window, popup, layer)| {
            let kind = if window.is_some() {
                VirtualOutputElementKind::Window
            } else if popup.is_some() {
                VirtualOutputElementKind::Popup
            } else if layer.is_some() {
                VirtualOutputElementKind::Layer
            } else {
                VirtualOutputElementKind::Unknown
            };
            (surface.id, (geometry.clone(), kind))
        })
        .collect::<HashMap<_, _>>();

    let mut elements = Vec::with_capacity(render_list.elements.len());
    for render_element in &render_list.elements {
        if render_element.surface_id == 0 {
            let (x, y) = pointer
                .as_ref()
                .map(|pointer| (pointer.x.round() as i32, pointer.y.round() as i32))
                .unwrap_or((0, 0));
            elements.push(VirtualOutputElement {
                surface_id: 0,
                kind: VirtualOutputElementKind::Cursor,
                x,
                y,
                width: DEFAULT_CURSOR_SIZE,
                height: DEFAULT_CURSOR_SIZE,
                z_index: render_element.z_index,
                opacity: render_element.opacity,
            });
            continue;
        }

        let Some((geometry, kind)) = geometry_by_surface.get(&render_element.surface_id) else {
            continue;
        };

        elements.push(VirtualOutputElement {
            surface_id: render_element.surface_id,
            kind: *kind,
            x: geometry.x,
            y: geometry.y,
            width: geometry.width,
            height: geometry.height,
            z_index: render_element.z_index,
            opacity: render_element.opacity,
        });
    }

    capture_state.push_frame(VirtualOutputFrame {
        output_name: output.name.clone(),
        frame: clock.frame,
        uptime_millis: clock.uptime_millis.min(u128::from(u64::MAX)) as u64,
        width: properties.width,
        height: properties.height,
        scale: properties.scale,
        background_color: config
            .as_deref()
            .map(|config| config.background_color.clone())
            .unwrap_or_else(|| "#000000".to_owned()),
        elements,
    });
}
