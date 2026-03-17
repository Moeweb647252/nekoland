//! In-process integration test for the virtual backend's offscreen frame capture and presentation
//! timeline reporting.

use std::time::Duration;

use nekoland::build_app;
use nekoland_backend::{BackendStatus, traits::BackendKind};
use nekoland_core::app::RunLoopSettings;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    OutputDevice, OutputId, OutputKind, OutputProperties, SurfaceGeometry, WindowLayout,
    WindowMode, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    OutputPresentationState, PresentAuditState, VirtualOutputCaptureState, VirtualOutputElementKind,
};

mod common;

/// Surface id of the seeded floating window that the virtual backend should capture.
const TEST_SURFACE_ID: u64 = 4242;

/// Verifies that the virtual backend captures offscreen frames and advances the presentation
/// timeline for its synthetic output.
#[test]
fn virtual_backend_captures_offscreen_frames_and_presentation_timeline() {
    let _env_lock = common::env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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

    if let Err(error) = app.run() {
        panic!("virtual-output app should complete the configured frame budget: {error}");
    }

    let world = app.inner_mut().world_mut();
    let Some(backend_status) = world.get_resource::<BackendStatus>() else {
        panic!("backend status should remain available");
    };
    let backend_status = backend_status.clone();
    let output_state =
        world.query::<(&OutputId, &OutputDevice, &OutputProperties)>().iter(world).next().map(
            |(output_id, output, properties)| (*output_id, output.clone(), properties.clone()),
        );
    let Some((output_id, output, properties)) = output_state else {
        panic!("virtual backend should publish one output");
    };
    let Some(capture_state) = world.get_resource::<VirtualOutputCaptureState>() else {
        panic!("virtual output capture state should be available");
    };
    let Some(present_audit) = world.get_resource::<PresentAuditState>() else {
        panic!("present audit state should be available");
    };
    let Some(presentation_state) = world.get_resource::<OutputPresentationState>() else {
        panic!("output presentation state should be available");
    };

    assert!(
        backend_status.active.iter().any(|backend| backend.kind == BackendKind::Virtual),
        "virtual backend should remain active: {backend_status:?}"
    );
    assert_eq!(output.kind, OutputKind::Virtual);
    assert!(!capture_state.frames.is_empty(), "virtual backend should capture at least one frame");

    let Some(latest_frame) = capture_state.frames.back() else {
        panic!("virtual backend should retain the latest offscreen frame");
    };
    assert_eq!(latest_frame.output_name, output.name);
    assert_eq!(latest_frame.width, properties.width);
    assert_eq!(latest_frame.height, properties.height);
    assert_eq!(latest_frame.scale, properties.scale);
    assert_eq!(latest_frame.background_color, "#f5f7ff");

    let window = latest_frame
        .elements
        .iter()
        .find(|element| element.surface_id == TEST_SURFACE_ID)
        .unwrap_or_else(|| panic!("virtual frame should include the seeded floating window"));
    assert_eq!(window.kind, VirtualOutputElementKind::Window);
    assert_eq!(window.x, 64);
    assert_eq!(window.y, 48);
    assert_eq!(window.width, 400);
    assert_eq!(window.height, 240);
    let audit = present_audit
        .outputs
        .get(&output_id)
        .unwrap_or_else(|| panic!("present audit should publish the active output"));
    assert_eq!(audit.output_name, output.name);
    assert_eq!(
        audit
            .elements
            .iter()
            .find(|element| element.surface_id == TEST_SURFACE_ID)
            .map(|element| (element.x, element.y, element.width, element.height)),
        Some((64, 48, 400, 240))
    );

    let presentation = presentation_state
        .outputs
        .iter()
        .find(|timeline| timeline.output_id == output_id)
        .unwrap_or_else(|| {
            panic!("virtual backend should publish a presentation timeline for its output")
        });
    assert!(presentation.sequence > 0, "virtual output should advance presentation sequence");
}

/// Seeds one floating window so the virtual backend has a deterministic scene to capture.
fn seed_floating_window(world: &mut bevy_ecs::world::World) {
    world.spawn((WindowBundle {
        surface: WlSurfaceHandle { id: TEST_SURFACE_ID },
        geometry: SurfaceGeometry { x: 64, y: 48, width: 400, height: 240 },
        window: XdgWindow {
            app_id: "org.nekoland.virtual-output".to_owned(),
            title: "Virtual Output Window".to_owned(),
            last_acked_configure: None,
        },
        layout: WindowLayout::Floating,
        mode: WindowMode::Normal,
        ..Default::default()
    },));
}
