//! Integration regression for non-SHM import preparation flowing through the render subapp.

use std::collections::BTreeMap;

use nekoland::build_app;
use nekoland_core::prelude::RenderSubApp;
use nekoland_ecs::bundles::WindowBundle;
use nekoland_ecs::components::{
    OutputId, SurfaceGeometry, WindowLayout, WindowMode, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    CompiledOutputFrames, OutputGeometrySnapshot, OutputSnapshotState, PlatformDmabufFormat,
    PlatformSurfaceBufferSource, PlatformSurfaceImportStrategy, PlatformSurfaceKind,
    PlatformSurfaceSnapshot, PlatformSurfaceSnapshotState, PreparedGpuResources, ShellRenderInput,
    SurfacePresentationRole, SurfacePresentationSnapshot, SurfacePresentationState,
    UNASSIGNED_WORKSPACE_STACK_ID, WaylandIngress, WindowStackingState,
};
use smithay::backend::allocator::{Fourcc, Modifier};

mod common;

const TEST_SURFACE_ID: u64 = 4242;
const TEST_OUTPUT_ID: OutputId = OutputId(7);

#[test]
fn render_subapp_prepares_external_texture_imports_from_mailboxes() {
    let runtime_dir = common::RuntimeDirGuard::new("nekoland-external-texture-prepare");
    let config_path = common::write_default_config_with_xwayland_disabled(
        &runtime_dir.path,
        "external-texture.toml",
    );
    let mut app = build_app(config_path);

    {
        let world = app.inner_mut().world_mut();
        world.insert_resource(WindowStackingState {
            workspaces: BTreeMap::from([(UNASSIGNED_WORKSPACE_STACK_ID, vec![TEST_SURFACE_ID])]),
        });
        world.insert_resource(WaylandIngress {
            output_snapshots: OutputSnapshotState {
                outputs: vec![OutputGeometrySnapshot {
                    output_id: TEST_OUTPUT_ID,
                    name: "Virtual-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                    scale: 1,
                    refresh_millihz: 60_000,
                }],
            },
            surface_snapshots: PlatformSurfaceSnapshotState {
                surfaces: BTreeMap::from([(
                    TEST_SURFACE_ID,
                    PlatformSurfaceSnapshot {
                        surface_id: TEST_SURFACE_ID,
                        kind: PlatformSurfaceKind::Toplevel,
                        buffer_source: PlatformSurfaceBufferSource::DmaBuf,
                        dmabuf_format: Some(PlatformDmabufFormat {
                            code: Fourcc::Argb8888 as u32,
                            modifier: u64::from(Modifier::Linear),
                        }),
                        import_strategy: PlatformSurfaceImportStrategy::ExternalTextureImport,
                        attached: true,
                        scale: 1,
                        content_version: 9,
                    },
                )]),
            },
            ..WaylandIngress::default()
        });
        world.insert_resource(ShellRenderInput {
            surface_presentation: SurfacePresentationSnapshot {
                surfaces: BTreeMap::from([(
                    TEST_SURFACE_ID,
                    SurfacePresentationState {
                        visible: true,
                        target_output: Some(TEST_OUTPUT_ID),
                        geometry: SurfaceGeometry { x: 32, y: 24, width: 320, height: 200 },
                        input_enabled: true,
                        damage_enabled: true,
                        role: SurfacePresentationRole::Window,
                    },
                )]),
            },
            ..ShellRenderInput::default()
        });
        world.spawn((WindowBundle {
            surface: WlSurfaceHandle { id: TEST_SURFACE_ID },
            geometry: SurfaceGeometry { x: 32, y: 24, width: 320, height: 200 },
            window: XdgWindow {
                app_id: "org.nekoland.external-texture".to_owned(),
                title: "External Texture".to_owned(),
                last_acked_configure: None,
            },
            layout: WindowLayout::Floating,
            mode: WindowMode::Normal,
            ..Default::default()
        },));
    }

    {
        let mut render_subapp =
            app.inner_mut().remove_sub_app(RenderSubApp).expect("render subapp");
        render_subapp.extract(app.inner_mut().world_mut());
        render_subapp.update();
        app.inner_mut().insert_sub_app(RenderSubApp, render_subapp);
    }

    let render_world = app.inner().sub_app(RenderSubApp).world();
    let prepared = render_world.resource::<PreparedGpuResources>();
    let compiled = render_world.resource::<CompiledOutputFrames>();

    let prepared_import = prepared
        .surface_imports
        .get(&TEST_SURFACE_ID)
        .unwrap_or_else(|| panic!("prepared gpu imports should include test surface"));
    assert_eq!(
        prepared_import.strategy,
        nekoland_ecs::resources::PreparedSurfaceImportStrategy::ExternalTextureImport
    );
    assert_eq!(
        prepared_import.cache_key.strategy,
        nekoland_ecs::resources::PreparedSurfaceImportStrategy::ExternalTextureImport
    );
    assert_eq!(prepared_import.cache_key.content_version, 9);

    let output_gpu = prepared
        .outputs
        .get(&TEST_OUTPUT_ID)
        .unwrap_or_else(|| panic!("prepared output gpu resources should include target output"));
    assert_eq!(
        output_gpu.surface_imports[&TEST_SURFACE_ID].cache_key.strategy,
        nekoland_ecs::resources::PreparedSurfaceImportStrategy::ExternalTextureImport
    );

    let compiled_import = compiled
        .output(TEST_OUTPUT_ID)
        .and_then(|frame| frame.gpu_prep.as_ref())
        .and_then(|gpu_prep| gpu_prep.surface_imports.get(&TEST_SURFACE_ID))
        .unwrap_or_else(|| panic!("compiled frame should carry prepared external-texture import"));
    assert_eq!(
        compiled_import.strategy,
        nekoland_ecs::resources::PreparedSurfaceImportStrategy::ExternalTextureImport
    );
    assert_eq!(compiled_import.cache_key.content_version, 9);
}
