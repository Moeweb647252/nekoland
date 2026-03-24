//! Heavier virtual-backend soak coverage for long-running multi-output presentation.

use std::collections::BTreeSet;
use std::time::Duration;

use nekoland::build_app;
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry, WindowLayout,
    WindowMode, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{PlatformBackendKind, WaylandFeedback, WaylandIngress};

mod common;

const TEST_SURFACE_ID: u64 = 9001;
const EXTRA_OUTPUT_TOML: &str = r#"
[[outputs]]
name = "HDMI-A-1"
mode = "1280x720@60"
scale = 1
enabled = true
"#;

#[test]
fn virtual_backend_soak_tracks_multi_output_presentation_timelines() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-virtual-multi-output-soak");
    let config_path = common::write_default_config_with_extra(
        &runtime_dir.path,
        "virtual-multi-output.toml",
        EXTRA_OUTPUT_TOML,
    );

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(192),
    });
    seed_floating_window(app.inner_mut().world_mut());

    if let Err(error) = app.run() {
        panic!(
            "virtual multi-output soak app should complete the configured frame budget: {error}"
        );
    }

    let world = app.inner_mut().world_mut();
    let outputs = world
        .query::<(&OutputId, &OutputDevice, &OutputProperties)>()
        .iter(world)
        .map(|(output_id, output, properties)| (*output_id, output.clone(), properties.clone()))
        .collect::<Vec<_>>();
    let Some(wayland_feedback) = world.get_resource::<WaylandFeedback>() else {
        panic!("wayland feedback should be available after soak run");
    };
    let Some(wayland_ingress) = world.get_resource::<WaylandIngress>() else {
        panic!("wayland ingress should be available after soak run");
    };

    assert_eq!(outputs.len(), 2, "virtual backend should materialize both configured outputs");
    assert!(
        wayland_feedback
            .platform_backends
            .active
            .iter()
            .any(|backend| backend.kind == PlatformBackendKind::Virtual),
        "virtual backend should remain active during soak run: {:?}",
        wayland_feedback.platform_backends
    );

    let output_names =
        outputs.iter().map(|(_, output, _)| output.name.clone()).collect::<BTreeSet<_>>();
    assert_eq!(
        output_names,
        BTreeSet::from(["HDMI-A-1".to_owned(), "eDP-1".to_owned()]),
        "multi-output soak should preserve both configured output names"
    );
    assert!(outputs.iter().all(|(_, output, _)| output.kind == OutputKind::Virtual));
    assert_eq!(wayland_ingress.output_snapshots.outputs.len(), 2);
    assert_eq!(wayland_feedback.present_audit.outputs.len(), 2);
    assert_eq!(wayland_feedback.output_presentation.outputs.len(), 2);

    for (output_id, output, properties) in &outputs {
        let audit = wayland_feedback
            .present_audit
            .outputs
            .get(output_id)
            .unwrap_or_else(|| panic!("present audit should include {}", output.name));
        assert_eq!(audit.output_name, output.name);
        let timeline = wayland_feedback
            .output_presentation
            .outputs
            .iter()
            .find(|timeline| timeline.output_id == *output_id)
            .unwrap_or_else(|| panic!("presentation timeline should include {}", output.name));
        assert!(
            timeline.sequence >= 16,
            "multi-output soak should advance presentation sequence for {} (got {})",
            output.name,
            timeline.sequence
        );
        assert_eq!(timeline.refresh_interval_nanos, 16_666_666);
        assert_eq!(
            wayland_ingress
                .output_snapshots
                .outputs
                .iter()
                .find(|snapshot| snapshot.output_id == *output_id)
                .map(|snapshot| (snapshot.name.clone(), snapshot.width, snapshot.height)),
            Some((output.name.clone(), properties.width, properties.height)),
        );
    }

    assert!(
        !wayland_feedback.virtual_output_capture.frames.is_empty(),
        "virtual backend should retain captured frames during soak run"
    );
    assert!(
        wayland_feedback
            .virtual_output_capture
            .frames
            .iter()
            .any(|frame| output_names.contains(&frame.output_name)),
        "captured frames should belong to one of the configured outputs"
    );
}

fn seed_floating_window(world: &mut bevy_ecs::world::World) {
    world.spawn((WindowBundle {
        surface: WlSurfaceHandle { id: TEST_SURFACE_ID },
        geometry: SurfaceGeometry { x: 64, y: 48, width: 400, height: 240 },
        window: XdgWindow {
            app_id: "org.nekoland.virtual-soak".to_owned(),
            title: "Virtual Soak Window".to_owned(),
        },
        layout: WindowLayout::Floating,
        mode: WindowMode::Normal,
        ..Default::default()
    },));
}
