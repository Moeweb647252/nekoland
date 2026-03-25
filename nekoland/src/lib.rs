#![warn(missing_docs)]

//! Top-level application wiring for the `nekoland` compositor executable.
//!
//! This crate stitches together the workspace plugins and the Wayland/render sub-app boundaries
//! into the default runnable compositor application.

use std::env;
use std::path::PathBuf;

use bevy_ecs::schedule::{InternedScheduleLabel, ScheduleLabel};
use bevy_ecs::system::RunSystemOnce;
use bevy_ecs::world::World;
use nekoland_backend::{
    BackendPlugin, BackendWaylandSubAppPlugin, extract_backend_wayland_subapp_inputs,
};
use nekoland_config::ConfigPlugin;
use nekoland_core::prelude::{NekolandApp, RenderSubApp, WaylandSubApp};
use nekoland_core::schedules::ExtractSchedule;
use nekoland_input::InputPlugin;
use nekoland_ipc::IpcPlugin;
use nekoland_protocol::{
    ProtocolPlugin, WaylandSubAppPlugin, extract_wayland_subapp_inputs, sync_wayland_subapp_back,
};
use nekoland_render::{
    RenderPlugin, RenderSubAppPlugin, configure_render_subapp, sync_render_subapp_back,
};
use nekoland_shell::ShellPlugin;

/// Resolves the default config path, allowing `NEKOLAND_CONFIG` to override the repository default.
pub fn default_config_path() -> PathBuf {
    env::var_os("NEKOLAND_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/default.toml"))
}

/// Builds the standard compositor application by wiring every crate plugin into the core app.
///
/// Cross-app ownership is intentionally explicit here:
/// - `wayland subapp` owns platform/runtime extraction and produces `WaylandIngress` /
///   `WaylandFeedback`
/// - `main app` owns shell policy and produces `ShellRenderInput` / `WaylandCommands`
/// - `render subapp` consumes `ShellRenderInput` plus normalized platform snapshots and exports
///   `CompiledOutputFrames`
pub fn build_app(config_path: impl Into<PathBuf>) -> NekolandApp {
    let mut app = NekolandApp::new("nekoland");
    app.add_plugin(ConfigPlugin::new(config_path.into()))
        .add_plugin(ProtocolPlugin)
        .add_plugin(BackendPlugin)
        .add_plugin(InputPlugin)
        .add_wayland_plugin(WaylandSubAppPlugin)
        .add_wayland_plugin(BackendWaylandSubAppPlugin)
        .add_plugin(ShellPlugin)
        .add_plugin(RenderPlugin)
        .add_render_plugin(RenderSubAppPlugin)
        .add_plugin(IpcPlugin);
    app.inner_mut().sub_app_mut(WaylandSubApp).set_extract(extract_combined_wayland_subapp_inputs);
    app.set_sub_app_sync_back(WaylandSubApp, sync_combined_wayland_subapp_back);
    configure_render_subapp(app.inner_mut().sub_app_mut(RenderSubApp));
    app.set_sub_app_sync_back(RenderSubApp, sync_render_subapp_back);
    app
}

/// Convenience wrapper that builds the app using the default config path resolution rules.
pub fn build_default_app() -> NekolandApp {
    build_app(default_config_path())
}

fn extract_combined_wayland_subapp_inputs(main_world: &mut World, wayland_world: &mut World) {
    // Pull shell-owned command boundaries and normalized ECS snapshots into the platform runtime.
    extract_wayland_subapp_inputs(main_world, wayland_world);
    extract_backend_wayland_subapp_inputs(main_world, wayland_world);
}

fn sync_combined_wayland_subapp_back(
    main_world: &mut World,
    wayland_world: &mut World,
    schedule: Option<InternedScheduleLabel>,
) {
    // Push only platform-owned boundary resources back into `main app`; backend-specific sync-back
    // should happen by enriching `WaylandIngress` / `WaylandFeedback`, not by special-case clones.
    sync_wayland_subapp_back(main_world, wayland_world, schedule);

    if schedule.is_none_or(|schedule| schedule == ExtractSchedule.intern())
        && let Err(error) = main_world
            .run_system_once(nekoland_backend::common::outputs::synchronize_backend_outputs_system)
    {
        tracing::error!(
            error = %error,
            "failed to apply backend output materialization during wayland sync-back"
        );
    }
}
