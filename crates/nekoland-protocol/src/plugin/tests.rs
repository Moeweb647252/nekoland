use std::any::TypeId;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use bevy_ecs::prelude::World;
use nekoland_ecs::bundles::OutputBundle;
use nekoland_ecs::components::{
    LayerShellSurface, OutputDevice, OutputId, OutputPlacement, OutputProperties, SurfaceGeometry,
    WindowViewportVisibility, WlSurfaceHandle, XdgWindow,
};
use nekoland_ecs::resources::{
    OutputGeometrySnapshot, OutputPresentationState, OutputPresentationTimeline, OutputRenderPlan,
    OutputSnapshotState, PlatformDmabufFormat, PlatformSurfaceBufferSource,
    PlatformSurfaceImportStrategy, RenderItemId, RenderItemIdentity, RenderItemInstance,
    RenderPlan, RenderPlanItem, RenderRect, RenderSceneRole, RenderSourceId, SurfaceRenderItem,
};
use smithay::backend::allocator::{Format as DmabufFormat, Fourcc, Modifier};
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::{
    decoration::zv1::client::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1},
    shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};

use super::DEFAULT_KEYBOARD_REPEAT_RATE;
use super::feedback::{current_output_presentation, current_output_timing};
use super::seat::{PointerFocusInputs, pointer_focus_target};
use super::server::{
    ForeignToplevelSnapshot, ProtocolClientState, ProtocolDmabufSupport, ProtocolRuntimeState,
    SmithayProtocolRuntime,
};
use crate::ProtocolEvent;
use nekoland_ecs::resources::XdgSurfaceRole;

#[derive(Debug)]
struct ClientSummary {
    globals: Vec<String>,
    configure_serial: u32,
    decoration_mode: Option<zxdg_toplevel_decoration_v1::Mode>,
    wm_capabilities: Vec<xdg_toplevel::WmCapabilities>,
}

#[derive(Debug, Default)]
struct TestClientState {
    globals: Vec<String>,
    base_surface: Option<wl_surface::WlSurface>,
    output: Option<wl_output::WlOutput>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    xdg_surface: Option<(xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel)>,
    xdg_decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    xdg_toplevel_decoration: Option<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1>,
    configure_serial: Option<u32>,
    decoration_mode: Option<zxdg_toplevel_decoration_v1::Mode>,
    wm_capabilities: Vec<xdg_toplevel::WmCapabilities>,
    request_fullscreen_on_first_configure: bool,
    sent_fullscreen_request: bool,
}

fn identity(id: u64) -> RenderItemIdentity {
    RenderItemIdentity::new(RenderSourceId(id), RenderItemId(id))
}

#[test]
fn protocol_resources_compat_facade_matches_ecs_shared_types() {
    assert_eq!(
        TypeId::of::<crate::resources::PendingXdgRequests>(),
        TypeId::of::<nekoland_ecs::resources::PendingXdgRequests>()
    );
    assert_eq!(
        TypeId::of::<crate::resources::PendingWindowServerRequests>(),
        TypeId::of::<nekoland_ecs::resources::PendingWindowServerRequests>()
    );
    assert_eq!(
        TypeId::of::<crate::resources::ClipboardSelectionState>(),
        TypeId::of::<nekoland_ecs::resources::ClipboardSelectionState>()
    );
    assert_eq!(
        TypeId::of::<crate::resources::OutputPresentationState>(),
        TypeId::of::<nekoland_ecs::resources::OutputPresentationState>()
    );
}

#[test]
fn protocol_root_facade_exports_remain_usable() {
    let mut sub_app = bevy_app::SubApp::new();
    crate::configure_wayland_subapp(&mut sub_app);

    let mut main_world = World::default();
    main_world.insert_resource(nekoland_ecs::resources::ShellRenderInput::default());
    let mut wayland_world = World::default();
    crate::extract_wayland_subapp_inputs(&mut main_world, &mut wayland_world);
    crate::sync_wayland_subapp_back(&mut main_world, &mut wayland_world, None);

    let _plugin = crate::ProtocolPlugin;
    let _subapp_plugin = crate::WaylandSubAppPlugin;
    let _dmabuf_support = crate::ProtocolDmabufSupport::default();
}

#[test]
fn platform_surface_import_strategy_marks_non_renderable_dmabufs_as_external_textures() {
    let format = DmabufFormat { code: Fourcc::Argb8888, modifier: Modifier::Linear };
    let dmabuf_format = PlatformDmabufFormat {
        code: Fourcc::Argb8888 as u32,
        modifier: u64::from(Modifier::Linear),
    };
    let support = ProtocolDmabufSupport {
        formats: vec![format],
        renderable_formats: Vec::new(),
        importable: true,
        main_device: None,
    };

    assert_eq!(
        super::surface::platform_surface_import_strategy(
            PlatformSurfaceBufferSource::DmaBuf,
            Some(dmabuf_format),
            Some(&support),
        ),
        PlatformSurfaceImportStrategy::ExternalTextureImport
    );
}

#[test]
fn platform_surface_import_strategy_keeps_renderable_dmabufs_on_direct_import_path() {
    let format = DmabufFormat { code: Fourcc::Argb8888, modifier: Modifier::Linear };
    let dmabuf_format = PlatformDmabufFormat {
        code: Fourcc::Argb8888 as u32,
        modifier: u64::from(Modifier::Linear),
    };
    let support = ProtocolDmabufSupport {
        formats: vec![format],
        renderable_formats: vec![format],
        importable: true,
        main_device: None,
    };

    assert_eq!(
        super::surface::platform_surface_import_strategy(
            PlatformSurfaceBufferSource::DmaBuf,
            Some(dmabuf_format),
            Some(&support),
        ),
        PlatformSurfaceImportStrategy::DmaBufImport
    );
}

#[test]
fn dmabuf_support_tracks_main_device_from_backend_feedback() {
    let format = DmabufFormat { code: Fourcc::Argb8888, modifier: Modifier::Linear };
    let mut support = ProtocolDmabufSupport::default();

    support.merge_formats([format], [format], true, Some(0xdead_beef));

    assert_eq!(support.main_device, Some(0xdead_beef));
    assert_eq!(support.formats, vec![format]);
    assert_eq!(support.renderable_formats, vec![format]);
    assert!(support.importable);
}

#[test]
fn output_timing_prefers_normalized_output_snapshots() {
    let timing = current_output_timing(Some(&OutputSnapshotState {
        outputs: vec![
            OutputGeometrySnapshot {
                output_id: OutputId(8),
                name: "DP-2".to_owned(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1,
                refresh_millihz: 60_000,
            },
            OutputGeometrySnapshot {
                output_id: OutputId(3),
                name: "DP-1".to_owned(),
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
                scale: 2,
                refresh_millihz: 144_000,
            },
        ],
    }))
    .expect("normalized output snapshots should produce timing");

    assert_eq!(timing.output_name, "DP-1");
    assert_eq!(timing.width, 2560);
    assert_eq!(timing.height, 1440);
    assert_eq!(timing.scale, 2);
    assert_eq!(timing.refresh_millihz, 144_000);
}

#[test]
fn output_presentation_uses_snapshot_ids_to_pick_feedback_timeline() {
    let timing = current_output_presentation(
        Some(&OutputSnapshotState {
            outputs: vec![
                OutputGeometrySnapshot {
                    output_id: OutputId(5),
                    name: "DP-2".to_owned(),
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    scale: 1,
                    refresh_millihz: 60_000,
                },
                OutputGeometrySnapshot {
                    output_id: OutputId(2),
                    name: "DP-1".to_owned(),
                    x: 1920,
                    y: 0,
                    width: 2560,
                    height: 1440,
                    scale: 2,
                    refresh_millihz: 144_000,
                },
            ],
        }),
        Some(&OutputPresentationState {
            outputs: vec![OutputPresentationTimeline {
                output_id: OutputId(2),
                sequence: 77,
                refresh_interval_nanos: 16_666_667,
                present_time_nanos: 123_456_789,
            }],
        }),
    )
    .expect("matching output snapshot and presentation timeline should produce timing");

    assert_eq!(timing.sequence, Some(77));
}

#[test]
fn roundtrip_exposes_globals_and_emits_toplevel_events() {
    let socket_path = temporary_socket_path();
    let listener = match UnixListener::bind(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            eprintln!("skipping protocol round-trip test in restricted sandbox: {error}");
            return;
        }
        Err(error) => panic!("test UnixListener bind: {error}"),
    };

    let (result_tx, result_rx) = mpsc::channel();
    let client_socket_path = socket_path.clone();
    let client_thread = thread::spawn(move || {
        let result = run_test_client(client_socket_path);
        let _ = result_tx.send(result);
    });
    let Ok((server_stream, _)) = listener.accept() else {
        panic!("test UnixListener accept");
    };
    let _ = fs::remove_file(&socket_path);
    let mut runtime = test_runtime(server_stream);

    let Some(summary) = pump_server_until_client_finishes(&mut runtime, &result_rx) else {
        let Ok(()) = client_thread.join() else {
            panic!("client thread should exit cleanly");
        };
        return;
    };
    let Ok(()) = client_thread.join() else {
        panic!("client thread should exit cleanly");
    };

    for _ in 0..4 {
        runtime.dispatch_clients();
        thread::sleep(Duration::from_millis(1));
    }

    let events = runtime.drain_events();
    let Some(surface_id) = events.iter().find_map(|event| match event {
        ProtocolEvent::ConfigureRequested { surface_id, role: XdgSurfaceRole::Toplevel } => {
            Some(*surface_id)
        }
        _ => None,
    }) else {
        panic!("server should emit a toplevel configure request");
    };

    assert_globals_present(&summary.globals);
    assert_eq!(
        summary.decoration_mode,
        Some(zxdg_toplevel_decoration_v1::Mode::ClientSide),
        "client should observe client-side decoration preference",
    );
    assert!(
        summary.wm_capabilities.contains(&xdg_toplevel::WmCapabilities::Maximize),
        "client should see maximize advertised in wm_capabilities: {:?}",
        summary.wm_capabilities
    );
    assert!(
        summary.wm_capabilities.contains(&xdg_toplevel::WmCapabilities::Minimize),
        "client should see minimize advertised in wm_capabilities: {:?}",
        summary.wm_capabilities
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProtocolEvent::SurfaceCommitted {
                surface_id: event_surface_id,
                role: XdgSurfaceRole::Toplevel,
                ..
            } if *event_surface_id == surface_id
        )),
        "server should record the toplevel commit: {events:#?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProtocolEvent::AckConfigure {
                surface_id: event_surface_id,
                role: XdgSurfaceRole::Toplevel,
                serial,
            } if *event_surface_id == surface_id && *serial == summary.configure_serial
        )),
        "server should record the configure ack: {events:#?}"
    );
}

#[test]
fn pointer_hit_test_prefers_layer_surfaces_above_windows() {
    let mut world = World::default();
    world.spawn((
        OutputId(1),
        OutputDevice { name: "Virtual-1".to_owned(), ..Default::default() },
        OutputProperties { width: 320, height: 64, refresh_millihz: 60_000, scale: 1 },
        OutputPlacement { x: 0, y: 0 },
    ));
    world.spawn((
        WlSurfaceHandle { id: 11 },
        SurfaceGeometry { x: 0, y: 0, width: 320, height: 64 },
        XdgWindow::default(),
    ));
    world.spawn((
        WlSurfaceHandle { id: 22 },
        SurfaceGeometry { x: 0, y: 0, width: 320, height: 64 },
        LayerShellSurface::default(),
    ));

    let render_plan = RenderPlan {
        outputs: std::collections::BTreeMap::from([(
            OutputId(1),
            OutputRenderPlan::from_items([
                RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(11),
                    surface_id: 11,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 320, height: 64 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 0,
                        scene_role: RenderSceneRole::Desktop,
                    },
                }),
                RenderPlanItem::Surface(SurfaceRenderItem {
                    identity: identity(22),
                    surface_id: 22,
                    instance: RenderItemInstance {
                        rect: RenderRect { x: 0, y: 0, width: 320, height: 64 },
                        opacity: 1.0,
                        clip_rect: None,
                        z_index: 1,
                        scene_role: RenderSceneRole::Desktop,
                    },
                }),
            ]),
        )]),
    };
    let focus_inputs = PointerFocusInputs {
        render_plan: Some(&render_plan),
        surface_presentation: None,
        output_snapshots: Some(&OutputSnapshotState {
            outputs: vec![OutputGeometrySnapshot {
                output_id: OutputId(1),
                name: "Virtual-1".to_owned(),
                x: 0,
                y: 0,
                width: 320,
                height: 64,
                scale: 1,
                refresh_millihz: 60_000,
            }],
        }),
    };

    let Some(target) = pointer_focus_target(16.0, 16.0, None, (16.0, 16.0).into(), &focus_inputs)
    else {
        panic!("pointer focus target should exist");
    };

    assert_eq!(target.surface_id, 22);
    assert_eq!(target.surface_origin, (0.0, 0.0).into());
}

#[test]
fn pointer_hit_test_offsets_output_local_window_geometry_by_output_placement() {
    let mut world = World::default();
    world.spawn(OutputBundle {
        output: OutputDevice { name: "DP-1".to_owned(), ..Default::default() },
        properties: OutputProperties { width: 100, height: 100, refresh_millihz: 60_000, scale: 1 },
        placement: OutputPlacement { x: 0, y: 0 },
        ..Default::default()
    });
    world.spawn(OutputBundle {
        output: OutputDevice { name: "DP-2".to_owned(), ..Default::default() },
        properties: OutputProperties { width: 100, height: 100, refresh_millihz: 60_000, scale: 1 },
        placement: OutputPlacement { x: 100, y: 0 },
        ..Default::default()
    });
    let dp2_id = world
        .query::<(&OutputId, &OutputDevice)>()
        .iter(&world)
        .find(|(_, output)| output.name == "DP-2")
        .map(|(output_id, _)| *output_id)
        .expect("dp-2 output id");
    world.spawn((
        WlSurfaceHandle { id: 42 },
        SurfaceGeometry { x: 0, y: 0, width: 80, height: 80 },
        WindowViewportVisibility { visible: true, output: Some(dp2_id) },
        XdgWindow::default(),
    ));
    let render_plan = RenderPlan {
        outputs: std::collections::BTreeMap::from([(
            dp2_id,
            OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                identity: identity(42),
                surface_id: 42,
                instance: RenderItemInstance {
                    rect: RenderRect { x: 0, y: 0, width: 80, height: 80 },
                    opacity: 1.0,
                    clip_rect: None,
                    z_index: 0,
                    scene_role: RenderSceneRole::Desktop,
                },
            })]),
        )]),
    };
    let focus_inputs = PointerFocusInputs {
        render_plan: Some(&render_plan),
        surface_presentation: None,
        output_snapshots: Some(&OutputSnapshotState {
            outputs: vec![
                OutputGeometrySnapshot {
                    output_id: OutputId(1),
                    name: "DP-1".to_owned(),
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                    scale: 1,
                    refresh_millihz: 60_000,
                },
                OutputGeometrySnapshot {
                    output_id: dp2_id,
                    name: "DP-2".to_owned(),
                    x: 100,
                    y: 0,
                    width: 100,
                    height: 100,
                    scale: 1,
                    refresh_millihz: 60_000,
                },
            ],
        }),
    };

    let Some(target) = pointer_focus_target(110.0, 10.0, None, (110.0, 10.0).into(), &focus_inputs)
    else {
        panic!("window on the second output should receive pointer focus");
    };

    assert_eq!(target.surface_id, 42);
    assert_eq!(target.surface_origin, (100.0, 0.0).into());
    assert!(
        pointer_focus_target(10.0, 10.0, None, (10.0, 10.0).into(), &focus_inputs).is_none(),
        "output-local geometry should not be hit-tested at the wrong global origin",
    );
}

#[test]
fn pointer_hit_test_respects_render_item_clip_rect() {
    let mut world = World::default();
    let output_entity = world
        .spawn(OutputBundle {
            output: OutputDevice { name: "Virtual-1".to_owned(), ..Default::default() },
            properties: OutputProperties {
                width: 128,
                height: 128,
                refresh_millihz: 60_000,
                scale: 1,
            },
            placement: OutputPlacement { x: 0, y: 0 },
            ..Default::default()
        })
        .id();
    let output_id = *world.get::<OutputId>(output_entity).expect("virtual output id");
    world.spawn((
        WlSurfaceHandle { id: 77 },
        SurfaceGeometry { x: 0, y: 0, width: 80, height: 80 },
        XdgWindow::default(),
    ));

    let render_plan = RenderPlan {
        outputs: std::collections::BTreeMap::from([(
            output_id,
            OutputRenderPlan::from_items([RenderPlanItem::Surface(SurfaceRenderItem {
                identity: identity(77),
                surface_id: 77,
                instance: RenderItemInstance {
                    rect: RenderRect { x: 0, y: 0, width: 80, height: 80 },
                    opacity: 1.0,
                    clip_rect: Some(RenderRect { x: 40, y: 0, width: 40, height: 80 }),
                    z_index: 0,
                    scene_role: RenderSceneRole::Desktop,
                },
            })]),
        )]),
    };
    let focus_inputs = PointerFocusInputs {
        render_plan: Some(&render_plan),
        surface_presentation: None,
        output_snapshots: Some(&OutputSnapshotState {
            outputs: vec![OutputGeometrySnapshot {
                output_id,
                name: "Virtual-1".to_owned(),
                x: 0,
                y: 0,
                width: 128,
                height: 128,
                scale: 1,
                refresh_millihz: 60_000,
            }],
        }),
    };

    assert!(
        pointer_focus_target(10.0, 10.0, None, (10.0, 10.0).into(), &focus_inputs).is_none(),
        "clipped-out region should not receive pointer focus",
    );
    let Some(target) = pointer_focus_target(60.0, 10.0, None, (60.0, 10.0).into(), &focus_inputs)
    else {
        panic!("visible clipped region should still receive pointer focus");
    };
    assert_eq!(target.surface_id, 77);
    assert_eq!(target.surface_origin, (0.0, 0.0).into());
}

#[test]
fn foreign_toplevel_sync_creates_updates_and_removes_handles() {
    let socket_path = temporary_socket_path();
    let Ok(listener) = UnixListener::bind(&socket_path) else {
        panic!("test UnixListener bind");
    };
    let client_socket_path = socket_path.clone();
    let client_thread = thread::spawn(move || UnixStream::connect(client_socket_path));
    let Ok((server_stream, _)) = listener.accept() else {
        panic!("test UnixListener accept");
    };
    let _ = fs::remove_file(&socket_path);
    let mut runtime = test_runtime(server_stream);
    let _ = client_thread.join();

    runtime.sync_foreign_toplevel_list(&[ForeignToplevelSnapshot {
        surface_id: 11,
        title: "One".to_owned(),
        app_id: "app.one".to_owned(),
    }]);
    assert_eq!(runtime.state.foreign_toplevels.len(), 1);
    let Some(handle) = runtime.state.foreign_toplevels.get(&11) else {
        panic!("foreign toplevel handle should exist after sync");
    };
    assert_eq!(handle.title(), "One");
    assert_eq!(handle.app_id(), "app.one");

    runtime.sync_foreign_toplevel_list(&[ForeignToplevelSnapshot {
        surface_id: 11,
        title: "Renamed".to_owned(),
        app_id: "app.one".to_owned(),
    }]);
    let Some(handle) = runtime.state.foreign_toplevels.get(&11) else {
        panic!("foreign toplevel handle should still exist after update");
    };
    assert_eq!(handle.title(), "Renamed");

    runtime.sync_foreign_toplevel_list(&[]);
    assert!(runtime.state.foreign_toplevels.is_empty());
}

#[test]
fn fullscreen_request_uses_bound_output_name() {
    let socket_path = temporary_socket_path();
    let Ok(listener) = UnixListener::bind(&socket_path) else {
        panic!("test UnixListener bind");
    };
    let (result_tx, result_rx) = mpsc::channel();
    let client_socket_path = socket_path.clone();
    let client_thread = thread::spawn(move || {
        let result = run_fullscreen_test_client(client_socket_path);
        let _ = result_tx.send(result);
    });
    let Ok((server_stream, _)) = listener.accept() else {
        panic!("test UnixListener accept");
    };
    let _ = fs::remove_file(&socket_path);
    let mut runtime = test_runtime(server_stream);

    let Some(_) = pump_server_until_client_finishes(&mut runtime, &result_rx) else {
        let Ok(()) = client_thread.join() else {
            panic!("client thread should exit cleanly");
        };
        return;
    };
    let Ok(()) = client_thread.join() else {
        panic!("client thread should exit cleanly");
    };

    for _ in 0..4 {
        runtime.dispatch_clients();
        thread::sleep(Duration::from_millis(1));
    }

    assert!(
        runtime.drain_events().iter().any(|event| matches!(
            event,
            ProtocolEvent::FullscreenRequested {
                output_name: Some(output_name),
                ..
            } if output_name == "Nekoland-1"
        )),
        "fullscreen request should carry the bound output name",
    );
}

fn test_runtime(server_stream: UnixStream) -> SmithayProtocolRuntime {
    let Ok(display) = Display::new() else {
        panic!("server display");
    };
    let mut display_handle = display.handle();
    let state = ProtocolRuntimeState::new(
        &display_handle,
        DEFAULT_KEYBOARD_REPEAT_RATE,
        &nekoland_config::resources::ConfiguredKeyboardLayout::default(),
    );
    let Ok(client) = display_handle
        .insert_client(server_stream, std::sync::Arc::new(ProtocolClientState::default()))
    else {
        panic!("server client registration");
    };

    SmithayProtocolRuntime {
        display,
        state,
        xwayland_event_loop: None,
        socket: None,
        clients: vec![client],
        last_accept_error: None,
        last_dispatch_error: None,
        last_xwayland_error: None,
    }
}

fn run_test_client(socket_path: std::path::PathBuf) -> Result<ClientSummary, String> {
    run_configure_test_client(socket_path, false)
}

fn run_fullscreen_test_client(socket_path: std::path::PathBuf) -> Result<ClientSummary, String> {
    run_configure_test_client(socket_path, true)
}

fn run_configure_test_client(
    socket_path: std::path::PathBuf,
    request_fullscreen_on_first_configure: bool,
) -> Result<ClientSummary, String> {
    let stream = UnixStream::connect(&socket_path)
        .map_err(|error| format!("socket connect failed: {error}"))?;
    let conn =
        Connection::from_socket(stream).map_err(|error| format!("from_socket failed: {error}"))?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    conn.display().get_registry(&qh, ());

    let mut state = TestClientState { request_fullscreen_on_first_configure, ..Default::default() };
    let deadline = Instant::now() + Duration::from_secs(2);

    while state.configure_serial.is_none()
        || (state.xdg_toplevel_decoration.is_some() && state.decoration_mode.is_none())
        || (request_fullscreen_on_first_configure && !state.sent_fullscreen_request)
    {
        client_dispatch_once(&mut event_queue, &mut state)
            .map_err(|error| format!("client dispatch failed: {error}"))?;
        if Instant::now() >= deadline {
            return Err("timed out waiting for xdg_surface.configure".to_owned());
        }
    }

    event_queue.flush().map_err(|error| format!("final flush after configure failed: {error}"))?;

    Ok(ClientSummary {
        globals: state.globals,
        configure_serial: state
            .configure_serial
            .ok_or_else(|| "client never received xdg_surface.configure".to_owned())?,
        decoration_mode: state.decoration_mode,
        wm_capabilities: state.wm_capabilities,
    })
}

fn client_dispatch_once(
    event_queue: &mut EventQueue<TestClientState>,
    state: &mut TestClientState,
) -> Result<(), String> {
    event_queue
        .dispatch_pending(state)
        .map_err(|error| format!("dispatch_pending before read failed: {error}"))?;
    event_queue.flush().map_err(|error| format!("flush failed: {error}"))?;

    let Some(read_guard) = event_queue.prepare_read() else {
        return Ok(());
    };

    read_guard.read().map_err(|error| format!("socket read failed: {error}"))?;
    event_queue
        .dispatch_pending(state)
        .map_err(|error| format!("dispatch_pending after read failed: {error}"))?;
    Ok(())
}

fn pump_server_until_client_finishes(
    runtime: &mut SmithayProtocolRuntime,
    result_rx: &mpsc::Receiver<Result<ClientSummary, String>>,
) -> Option<ClientSummary> {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        runtime.dispatch_clients();

        match result_rx.try_recv() {
            Ok(Ok(summary)) => return Some(summary),
            Ok(Err(error)) if error.contains("Operation not permitted") => {
                eprintln!("skipping protocol round-trip test in restricted sandbox: {error}");
                return None;
            }
            Ok(Err(error)) => panic!("test client failed: {error}"),
            Err(mpsc::TryRecvError::Disconnected) => {
                panic!("test client exited without sending a result")
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }

        assert!(Instant::now() < deadline, "timed out waiting for the protocol round-trip");

        thread::sleep(Duration::from_millis(1));
    }
}

fn assert_globals_present(globals: &[String]) {
    for expected in [
        "wl_compositor",
        "wl_subcompositor",
        "xdg_wm_base",
        "ext_foreign_toplevel_list_v1",
        "xdg_activation_v1",
        "zxdg_decoration_manager_v1",
        "zwlr_layer_shell_v1",
        "wl_data_device_manager",
        "zwp_linux_dmabuf_v1",
        "wp_viewporter",
        "wp_fractional_scale_manager_v1",
        "wl_shm",
        "wl_seat",
        "wl_output",
        "zxdg_output_manager_v1",
        "wp_presentation",
    ] {
        assert!(
            globals.iter().any(|global| global == expected),
            "missing advertised global `{expected}` in {globals:?}"
        );
    }
}

fn temporary_socket_path() -> std::path::PathBuf {
    let mut path = env::temp_dir();
    let Ok(duration_since_epoch) =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
    else {
        panic!("system time should be after epoch");
    };
    let unique = duration_since_epoch.as_nanos();
    path.push(format!("nekoland-protocol-test-{}-{unique}.sock", std::process::id()));
    path
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            state.globals.push(interface.clone());

            match interface.as_str() {
                "wl_compositor" => {
                    let compositor =
                        registry.bind::<wl_compositor::WlCompositor, _, _>(name, 1, qh, ());
                    state.base_surface = Some(compositor.create_surface(qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 6, qh, ()));
                    state.maybe_init_toplevel(qh);
                }
                "zxdg_decoration_manager_v1" => {
                    state.xdg_decoration_manager = Some(
                        registry.bind::<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1, _, _>(
                            name,
                            1,
                            qh,
                            (),
                        ),
                    );
                    state.maybe_init_decoration(qh);
                }
                "wl_output" => {
                    state.output =
                        Some(registry.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClientState {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            state.configure_serial = Some(serial);
            xdg_surface.ack_configure(serial);
            if state.request_fullscreen_on_first_configure
                && !state.sent_fullscreen_request
                && let (Some((_, toplevel)), Some(output)) =
                    (state.xdg_surface.as_ref(), state.output.as_ref())
            {
                toplevel.set_fullscreen(Some(output));
                state.sent_fullscreen_request = true;
            }
            if let Some(surface) = state.base_surface.as_ref() {
                surface.commit();
            }
        }
    }
}

delegate_noop!(TestClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(TestClientState: ignore wl_output::WlOutput);
delegate_noop!(TestClientState: ignore wl_surface::WlSurface);
delegate_noop!(TestClientState: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for TestClientState {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::WmCapabilities { capabilities } = event {
            state.wm_capabilities = decode_xdg_wm_capabilities(&capabilities);
        }
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ()> for TestClientState {
    fn event(
        state: &mut Self,
        _decoration: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event
            && let wayland_client::WEnum::Value(mode) = mode
        {
            state.decoration_mode = Some(mode);
        }
    }
}

impl TestClientState {
    fn maybe_init_toplevel(&mut self, qh: &QueueHandle<Self>) {
        if self.base_surface.is_none() || self.wm_base.is_none() || self.xdg_surface.is_some() {
            return;
        }

        let Some(surface) = self.base_surface.as_ref().cloned() else {
            panic!("surface presence checked immediately above");
        };
        let Some(wm_base) = self.wm_base.as_ref() else {
            panic!("wm_base presence checked immediately above");
        };

        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg_surface.get_toplevel(qh, ());
        self.xdg_surface = Some((xdg_surface, toplevel));
        self.maybe_init_decoration(qh);
        surface.commit();
    }

    fn maybe_init_decoration(&mut self, qh: &QueueHandle<Self>) {
        if self.xdg_decoration_manager.is_none()
            || self.xdg_surface.is_none()
            || self.xdg_toplevel_decoration.is_some()
        {
            return;
        }

        let Some(manager) = self.xdg_decoration_manager.as_ref() else {
            panic!("decoration manager presence checked immediately above");
        };
        let Some((_, toplevel)) = self.xdg_surface.as_ref() else {
            panic!("xdg toplevel presence checked immediately above");
        };

        let decoration = manager.get_toplevel_decoration(toplevel, qh, ());
        decoration.set_mode(zxdg_toplevel_decoration_v1::Mode::ServerSide);
        self.xdg_toplevel_decoration = Some(decoration);
    }
}

fn decode_xdg_wm_capabilities(raw: &[u8]) -> Vec<xdg_toplevel::WmCapabilities> {
    raw.chunks_exact(4)
        .flat_map(TryInto::<[u8; 4]>::try_into)
        .map(u32::from_ne_bytes)
        .flat_map(xdg_toplevel::WmCapabilities::try_from)
        .collect::<Vec<_>>()
}
