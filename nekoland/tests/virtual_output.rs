use std::time::Duration;

use nekoland::build_app;
use nekoland_backend::traits::{BackendKind, SelectedBackend};
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    LayoutSlot, OutputDevice, OutputKind, OutputProperties, SurfaceGeometry, WindowState,
    WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    OutputPresentationState, VirtualOutputCaptureState, VirtualOutputElementKind,
};

mod common;

const TEST_SURFACE_ID: u64 = 4242;

#[test]
fn virtual_backend_captures_offscreen_frames_and_presentation_timeline() {
    let _env_lock = common::env_lock().lock().expect("environment lock should not be poisoned");
    let _backend_guard = common::EnvVarGuard::set("NEKOLAND_BACKEND", "virtual");
    let _startup_guard = common::EnvVarGuard::set("NEKOLAND_DISABLE_STARTUP_COMMANDS", "1");
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-virtual-output");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "virtual-output.toml",
    );

    let mut app = build_app(config_path);
    app.insert_resource(RunLoopSettings {
        frame_timeout: Duration::from_millis(1),
        max_frames: Some(12),
    });
    seed_floating_window(app.inner_mut().world_mut());

    app.run().expect("virtual-output app should complete the configured frame budget");

    let world = app.inner_mut().world_mut();
    let selected_backend_kind = world
        .get_resource::<SelectedBackend>()
        .expect("selected backend should remain available")
        .kind
        .clone();
    let (output, properties) = world
        .query::<(&OutputDevice, &OutputProperties)>()
        .iter(world)
        .next()
        .map(|(output, properties)| (output.clone(), properties.clone()))
        .expect("virtual backend should publish one output");
    let capture_state = world
        .get_resource::<VirtualOutputCaptureState>()
        .expect("virtual output capture state should be available");
    let presentation_state = world
        .get_resource::<OutputPresentationState>()
        .expect("output presentation state should be available");

    assert_eq!(selected_backend_kind, BackendKind::Virtual);
    assert_eq!(output.kind, OutputKind::Virtual);
    assert!(!capture_state.frames.is_empty(), "virtual backend should capture at least one frame");

    let latest_frame = capture_state
        .frames
        .back()
        .expect("virtual backend should retain the latest offscreen frame");
    assert_eq!(latest_frame.output_name, output.name);
    assert_eq!(latest_frame.width, properties.width);
    assert_eq!(latest_frame.height, properties.height);
    assert_eq!(latest_frame.scale, properties.scale);
    assert_eq!(latest_frame.background_color, "#f5f7ff");

    let window = latest_frame
        .elements
        .iter()
        .find(|element| element.surface_id == TEST_SURFACE_ID)
        .expect("virtual frame should include the seeded floating window");
    assert_eq!(window.kind, VirtualOutputElementKind::Window);
    assert_eq!(window.x, 64);
    assert_eq!(window.y, 48);
    assert_eq!(window.width, 400);
    assert_eq!(window.height, 240);

    let presentation = presentation_state
        .outputs
        .iter()
        .find(|timeline| timeline.output_name == output.name)
        .expect("virtual backend should publish a presentation timeline for its output");
    assert!(presentation.sequence > 0, "virtual output should advance presentation sequence");
}

fn seed_floating_window(world: &mut bevy_ecs::world::World) {
    world.spawn((
        WindowBundle {
            surface: WlSurfaceHandle { id: TEST_SURFACE_ID },
            geometry: SurfaceGeometry { x: 64, y: 48, width: 400, height: 240 },
            window: XdgWindow {
                app_id: "org.nekoland.virtual-output".to_owned(),
                title: "Virtual Output Window".to_owned(),
                last_acked_configure: None,
            },
            state: WindowState::Floating,
            ..Default::default()
        },
        LayoutSlot { workspace: 1, column: 0, row: 0 },
    ));
}
