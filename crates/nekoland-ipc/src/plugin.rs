use bevy_app::App;
use bevy_ecs::schedule::IntoScheduleConfigs;
use nekoland_core::plugin::NekolandPlugin;
use nekoland_core::schedules::{ExtractSchedule, RenderSchedule};
use nekoland_ecs::resources::{
    PendingOutputServerRequests, PendingPopupServerRequests, PendingWindowServerRequests,
    PendingWorkspaceServerRequests,
};

use crate::{server, subscribe};

#[derive(Debug, Default, Clone, Copy)]
pub struct IpcPlugin;

impl NekolandPlugin for IpcPlugin {
    fn build(&self, app: &mut App) {
        let (runtime, server_state) = server::IpcServerRuntime::new();

        app.insert_resource(server_state)
            .insert_resource(server::IpcQueryCache::default())
            .insert_non_send_resource(runtime)
            .init_resource::<subscribe::PendingSubscriptionEvents>()
            .init_resource::<PendingPopupServerRequests>()
            .init_resource::<PendingWindowServerRequests>()
            .init_resource::<PendingWorkspaceServerRequests>()
            .init_resource::<PendingOutputServerRequests>()
            .add_systems(ExtractSchedule, server::accept_connections_system)
            .add_systems(
                RenderSchedule,
                (server::refresh_query_cache_system, subscribe::subscription_dispatch_system)
                    .chain(),
            );
    }
}
