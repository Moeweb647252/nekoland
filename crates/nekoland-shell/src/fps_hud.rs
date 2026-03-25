use bevy_ecs::prelude::{Res, ResMut};
use nekoland_config::resources::CompositorConfig;
use nekoland_ecs::resources::{
    CompositorClock, FpsHudMetricValue, FpsHudRuntimeState, OverlayUiFrame, OverlayUiLayer,
    RenderColor, RenderRect, WaylandFeedback, WaylandIngress,
};

const HUD_PANEL_ID: &str = "fps_hud.panel";
const HUD_PRESENT_ID: &str = "fps_hud.present";
const HUD_LOOP_ID: &str = "fps_hud.loop";
const HUD_X: i32 = 16;
const HUD_Y: i32 = 16;
const HUD_WIDTH: u32 = 164;
const HUD_HEIGHT: u32 = 56;
const HUD_TEXT_X: i32 = 28;
const HUD_PRESENT_Y: i32 = 24;
const HUD_LOOP_Y: i32 = 42;
const HUD_FONT_SIZE: f32 = 14.0;
const HUD_Z_INDEX: i32 = 10_000;
const HUD_PANEL_OPACITY: f32 = 0.72;
const HUD_TEXT_OPACITY: f32 = 1.0;
const HUD_PANEL_COLOR: RenderColor = RenderColor { r: 12, g: 14, b: 18, a: 255 };
const HUD_TEXT_COLOR: RenderColor = RenderColor { r: 244, g: 245, b: 246, a: 255 };

pub fn emit_fps_hud_system(
    clock: Option<Res<'_, CompositorClock>>,
    config: Option<Res<'_, CompositorConfig>>,
    wayland_ingress: Res<'_, WaylandIngress>,
    wayland_feedback: Res<'_, WaylandFeedback>,
    mut fps_hud_state: ResMut<'_, FpsHudRuntimeState>,
    mut overlay_ui: ResMut<'_, OverlayUiFrame>,
) {
    let Some(clock) = clock else {
        return;
    };

    fps_hud_state.observe_loop_clock(&clock);
    fps_hud_state
        .observe_output_presentation(&wayland_feedback.output_presentation, clock.uptime_millis);

    let config_enabled = config.as_deref().map_or(false, |config| config.debug.fps_hud);
    if !fps_hud_state.effective_enabled(config_enabled) {
        return;
    }

    let Some(output_id) = wayland_ingress.primary_output.id else {
        return;
    };

    let present_label =
        format_metric_line("Present", fps_hud_state.present_fps(output_id, clock.uptime_millis));
    let loop_label = format_metric_line("Loop", fps_hud_state.loop_fps());

    overlay_ui
        .output(output_id)
        .panel(
            HUD_PANEL_ID,
            OverlayUiLayer::Foreground,
            RenderRect { x: HUD_X, y: HUD_Y, width: HUD_WIDTH, height: HUD_HEIGHT },
            None,
            HUD_PANEL_COLOR,
            HUD_PANEL_OPACITY,
            HUD_Z_INDEX,
        )
        .text(
            HUD_PRESENT_ID,
            OverlayUiLayer::Foreground,
            HUD_TEXT_X,
            HUD_PRESENT_Y,
            None,
            present_label,
            HUD_FONT_SIZE,
            HUD_TEXT_COLOR,
            HUD_TEXT_OPACITY,
            HUD_Z_INDEX + 1,
        )
        .text(
            HUD_LOOP_ID,
            OverlayUiLayer::Foreground,
            HUD_TEXT_X,
            HUD_LOOP_Y,
            None,
            loop_label,
            HUD_FONT_SIZE,
            HUD_TEXT_COLOR,
            HUD_TEXT_OPACITY,
            HUD_Z_INDEX + 1,
        );
}

fn format_metric_line(label: &str, metric: FpsHudMetricValue) -> String {
    let value = match metric {
        FpsHudMetricValue::Unavailable => "--".to_owned(),
        FpsHudMetricValue::Fps(fps) => fps.to_string(),
    };
    format!("{label} {value}")
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use bevy_ecs::system::{IntoSystem, System};
    use nekoland_config::resources::{CompositorConfig, DebugConfig};
    use nekoland_ecs::components::OutputId;
    use nekoland_ecs::resources::{
        CompositorClock, FpsHudRuntimeState, OutputPresentationState, OutputPresentationTimeline,
        OverlayUiFrame, OverlayUiPrimitive, PrimaryOutputState, WaylandFeedback, WaylandIngress,
    };

    use super::emit_fps_hud_system;

    #[test]
    fn fps_hud_emits_two_line_metric_on_primary_output() {
        let mut world = World::default();
        world.insert_resource(CompositorConfig {
            debug: DebugConfig { fps_hud: true },
            ..CompositorConfig::default()
        });
        world.insert_resource(CompositorClock { frame: 1, uptime_millis: 0 });
        world.insert_resource(WaylandIngress {
            primary_output: PrimaryOutputState { id: Some(OutputId(7)) },
            ..WaylandIngress::default()
        });
        world.insert_resource(WaylandFeedback {
            output_presentation: OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id: OutputId(7),
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 0,
                    sequence: 1,
                }],
            },
            ..WaylandFeedback::default()
        });
        world.insert_resource(FpsHudRuntimeState::default());
        world.insert_resource(OverlayUiFrame::default());

        let mut system = IntoSystem::into_system(emit_fps_hud_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        world.insert_resource(CompositorClock { frame: 31, uptime_millis: 500 });
        world.insert_resource(WaylandFeedback {
            output_presentation: OutputPresentationState {
                outputs: vec![OutputPresentationTimeline {
                    output_id: OutputId(7),
                    refresh_interval_nanos: 16_666_667,
                    present_time_nanos: 500_000_000,
                    sequence: 31,
                }],
            },
            ..WaylandFeedback::default()
        });
        let _ = system.run((), &mut world);

        let output = world
            .resource::<OverlayUiFrame>()
            .outputs
            .get(&OutputId(7))
            .expect("HUD should target the primary output");
        assert_eq!(output.primitives.len(), 3);

        let mut texts = output
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                OverlayUiPrimitive::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        texts.sort();
        assert_eq!(texts, vec!["Loop 60".to_owned(), "Present 60".to_owned()]);
    }

    #[test]
    fn fps_hud_runtime_override_can_enable_disabled_config() {
        let mut world = World::default();
        world.insert_resource(CompositorConfig::default());
        world.insert_resource(CompositorClock { frame: 1, uptime_millis: 0 });
        world.insert_resource(WaylandIngress {
            primary_output: PrimaryOutputState { id: Some(OutputId(9)) },
            ..WaylandIngress::default()
        });
        world.insert_resource(WaylandFeedback::default());
        world.insert_resource(FpsHudRuntimeState::default());
        world.insert_resource(OverlayUiFrame::default());

        let mut system = IntoSystem::into_system(emit_fps_hud_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);
        assert!(world.resource::<OverlayUiFrame>().outputs.is_empty());

        world.resource_mut::<FpsHudRuntimeState>().override_enabled = Some(true);
        world.insert_resource(CompositorClock { frame: 2, uptime_millis: 16 });
        let _ = system.run((), &mut world);
        assert!(world.resource::<OverlayUiFrame>().outputs.contains_key(&OutputId(9)));
    }

    #[test]
    fn fps_hud_only_targets_the_wayland_primary_output() {
        let mut world = World::default();
        world.insert_resource(CompositorConfig {
            debug: DebugConfig { fps_hud: true },
            ..CompositorConfig::default()
        });
        world.insert_resource(CompositorClock { frame: 1, uptime_millis: 0 });
        world.insert_resource(WaylandIngress {
            primary_output: PrimaryOutputState { id: Some(OutputId(2)) },
            ..WaylandIngress::default()
        });
        world.insert_resource(WaylandFeedback {
            output_presentation: OutputPresentationState {
                outputs: vec![
                    OutputPresentationTimeline {
                        output_id: OutputId(2),
                        refresh_interval_nanos: 16_666_667,
                        present_time_nanos: 0,
                        sequence: 1,
                    },
                    OutputPresentationTimeline {
                        output_id: OutputId(3),
                        refresh_interval_nanos: 16_666_667,
                        present_time_nanos: 0,
                        sequence: 1,
                    },
                ],
            },
            ..WaylandFeedback::default()
        });
        world.insert_resource(FpsHudRuntimeState::default());
        world.insert_resource(OverlayUiFrame::default());

        let mut system = IntoSystem::into_system(emit_fps_hud_system);
        system.initialize(&mut world);
        let _ = system.run((), &mut world);

        let outputs = &world.resource::<OverlayUiFrame>().outputs;
        assert_eq!(outputs.len(), 1);
        assert!(outputs.contains_key(&OutputId(2)));
    }
}
