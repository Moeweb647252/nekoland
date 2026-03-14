use std::collections::HashMap;

use nekoland_ecs::resources::{RenderElement, RenderList};

use crate::traits::RenderSurfaceSnapshot;

/// Smithay expects render elements in front-to-back presentation order.
/// Our `RenderList` is composed back-to-front, so backend renderers must flip it here.
pub(crate) fn output_surfaces_in_presentation_order<'a>(
    render_list: &'a RenderList,
    surfaces: &'a HashMap<u64, RenderSurfaceSnapshot>,
    output_name: &'a str,
) -> impl Iterator<Item = (&'a RenderElement, &'a RenderSurfaceSnapshot)> + 'a {
    render_list.elements.iter().rev().filter_map(move |render_element| {
        let surface = surfaces.get(&render_element.surface_id)?;
        if surface.target_output.as_ref().is_some_and(|target_output| target_output != output_name)
        {
            return None;
        }

        Some((render_element, surface))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nekoland_ecs::components::SurfaceGeometry;
    use nekoland_ecs::resources::{RenderElement, RenderList};

    use crate::traits::{RenderSurfaceRole, RenderSurfaceSnapshot};

    use super::output_surfaces_in_presentation_order;

    #[test]
    fn presentation_order_flips_back_to_front_render_list() {
        let render_list = RenderList {
            elements: vec![
                RenderElement { surface_id: 11, z_index: 0, opacity: 1.0 },
                RenderElement { surface_id: 22, z_index: 1, opacity: 1.0 },
                RenderElement { surface_id: 33, z_index: 2, opacity: 1.0 },
            ],
        };
        let surfaces = HashMap::from([
            (
                11,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry::default(),
                    role: RenderSurfaceRole::Window,
                    target_output: None,
                },
            ),
            (
                22,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry::default(),
                    role: RenderSurfaceRole::Window,
                    target_output: Some("HDMI-A-1".to_owned()),
                },
            ),
            (
                33,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry::default(),
                    role: RenderSurfaceRole::Window,
                    target_output: Some("HDMI-A-1".to_owned()),
                },
            ),
        ]);

        let ordered = output_surfaces_in_presentation_order(&render_list, &surfaces, "HDMI-A-1")
            .map(|(render_element, _)| render_element.surface_id)
            .collect::<Vec<_>>();

        assert_eq!(ordered, vec![33, 22, 11]);
    }

    #[test]
    fn presentation_order_respects_output_routing() {
        let render_list = RenderList {
            elements: vec![
                RenderElement { surface_id: 11, z_index: 0, opacity: 1.0 },
                RenderElement { surface_id: 22, z_index: 1, opacity: 1.0 },
                RenderElement { surface_id: 33, z_index: 2, opacity: 1.0 },
            ],
        };
        let surfaces = HashMap::from([
            (
                11,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry::default(),
                    role: RenderSurfaceRole::Window,
                    target_output: Some("HDMI-A-1".to_owned()),
                },
            ),
            (
                22,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry::default(),
                    role: RenderSurfaceRole::Window,
                    target_output: Some("DP-1".to_owned()),
                },
            ),
            (
                33,
                RenderSurfaceSnapshot {
                    geometry: SurfaceGeometry::default(),
                    role: RenderSurfaceRole::Window,
                    target_output: None,
                },
            ),
        ]);

        let ordered = output_surfaces_in_presentation_order(&render_list, &surfaces, "HDMI-A-1")
            .map(|(render_element, _)| render_element.surface_id)
            .collect::<Vec<_>>();

        assert_eq!(ordered, vec![33, 11]);
    }
}
