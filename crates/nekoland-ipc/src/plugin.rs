use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_config::resources::KeyboardLayoutState;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{ExtractSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    PendingExternalCommandRequests, PendingOutputControls, PendingPopupServerRequests,
    PendingWindowControls, PendingWorkspaceControls,
};

use crate::{server, subscribe};

#[derive(Debug, Default, Clone, Copy)]
pub struct IpcPlugin;

impl NekolandPlugin for IpcPlugin {
    /// Register the IPC runtime plus the extract/render systems that accept
    /// client requests, rebuild query snapshots, and emit subscription events.
    fn build(&self, app: &mut App) {
        let (runtime, server_state) = server::IpcServerRuntime::new();

        app.insert_resource(server_state)
            .insert_resource(server::IpcQueryCache::default())
            .insert_non_send_resource(runtime)
            .init_resource::<KeyboardLayoutState>()
            .init_resource::<subscribe::PendingSubscriptionEvents>()
            .init_resource::<PendingExternalCommandRequests>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<PendingWindowControls>()
            .init_resource::<PendingWorkspaceControls>()
            .init_resource::<PendingOutputControls>()
            // Accept/process IPC input during Extract, then rebuild query snapshots and emit
            // subscription diffs after the render tree for the frame is known.
            .add_systems(ExtractSchedule, server::accept_connections_system)
            .add_systems(
                RenderSchedule,
                (server::refresh_query_cache_system, subscribe::subscription_dispatch_system)
                    .chain(),
            );
    }
}
