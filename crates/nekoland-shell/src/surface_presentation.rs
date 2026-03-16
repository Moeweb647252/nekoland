use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy_ecs::prelude::{Entity, Query, Res, ResMut, With};
use nekoland_ecs::components::{
    BufferState, DesiredOutputName, LayerOnOutput, LayerShellSurface, SurfaceGeometry,
    WlSurfaceHandle, XdgPopup, XdgWindow,
};
use nekoland_ecs::presentation_logic::{
    layer_visible, managed_window_visible, output_background_window_visible, popup_visible,
};
use nekoland_ecs::resources::{
    PrimaryOutputState, SurfacePresentationRole, SurfacePresentationSnapshot,
    SurfacePresentationState,
};
use nekoland_ecs::views::{OutputRuntime, PopupSnapshotRuntime, WindowSnapshotRuntime};

type LayerPresentationQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static WlSurfaceHandle,
        &'static SurfaceGeometry,
        &'static BufferState,
        Option<&'static LayerOnOutput>,
        Option<&'static DesiredOutputName>,
    ),
    With<LayerShellSurface>,
>;

pub fn surface_presentation_snapshot_system(
    outputs: Query<(Entity, OutputRuntime)>,
    primary_output: Option<Res<PrimaryOutputState>>,
    windows: Query<(Entity, WindowSnapshotRuntime), With<XdgWindow>>,
    popups: Query<PopupSnapshotRuntime, With<XdgPopup>>,
    layers: LayerPresentationQuery<'_, '_>,
    mut snapshot: ResMut<SurfacePresentationSnapshot>,
) {
    let live_output_names =
        outputs.iter().map(|(_, output)| output.name().to_owned()).collect::<BTreeSet<_>>();
    let output_names_by_entity = outputs
        .iter()
        .map(|(entity, output)| (entity, output.name().to_owned()))
        .collect::<HashMap<_, _>>();
    let primary_output_name = primary_output
        .and_then(|primary_output| primary_output.name.clone())
        .or_else(|| live_output_names.iter().next().cloned());

    let mut surfaces = BTreeMap::new();
    let mut window_presentation_by_entity = HashMap::new();

    for (entity, window) in windows.iter() {
        let role = if window.role.is_output_background() {
            SurfacePresentationRole::OutputBackground
        } else {
            SurfacePresentationRole::Window
        };
        let target_output = window
            .background
            .map(|background| background.output.clone())
            .or_else(|| window.viewport_visibility.output.clone())
            .filter(|output_name| live_output_names.contains(output_name));
        let visible = match role {
            SurfacePresentationRole::OutputBackground => output_background_window_visible(
                *window.mode,
                target_output.is_some(),
                *window.role,
            ),
            SurfacePresentationRole::Window => managed_window_visible(
                *window.mode,
                window.viewport_visibility.visible,
                *window.role,
            ),
            _ => false,
        };
        let state = SurfacePresentationState {
            visible,
            target_output: target_output.clone(),
            geometry: window.geometry.clone(),
            input_enabled: visible
                && window.role.is_managed()
                && window.x11_window.is_none_or(|window| !window.is_helper_surface()),
            damage_enabled: visible,
            role,
        };
        window_presentation_by_entity.insert(entity, state.clone());
        surfaces.insert(window.surface_id(), state);
    }

    for popup in popups.iter() {
        let parent_state = window_presentation_by_entity.get(&popup.child_of.parent());
        let visible =
            popup_visible(popup.buffer.attached, parent_state.is_some_and(|parent| parent.visible));
        surfaces.insert(
            popup.surface_id(),
            SurfacePresentationState {
                visible,
                target_output: parent_state.and_then(|parent| parent.target_output.clone()),
                geometry: popup.geometry.clone(),
                input_enabled: visible,
                damage_enabled: visible,
                role: SurfacePresentationRole::Popup,
            },
        );
    }

    for (surface, geometry, buffer, layer_output, desired_output_name) in layers.iter() {
        let target_output = layer_output
            .and_then(|layer_output| output_names_by_entity.get(&layer_output.0).cloned())
            .or_else(|| {
                desired_output_name.and_then(|desired_output_name| desired_output_name.0.clone())
            })
            .or_else(|| primary_output_name.clone())
            .filter(|output_name| live_output_names.contains(output_name));
        let visible = layer_visible(buffer.attached, target_output.is_some());
        surfaces.insert(
            surface.id,
            SurfacePresentationState {
                visible,
                target_output,
                geometry: (*geometry).clone(),
                input_enabled: visible,
                damage_enabled: visible,
                role: SurfacePresentationRole::Layer,
            },
        );
    }

    snapshot.surfaces = surfaces;
}
