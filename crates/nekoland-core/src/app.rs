use std::time::{Duration, Instant};

use bevy_app::{App, AppLabel, InternedAppLabel, SubApp};
use bevy_ecs::error::warn;
use bevy_ecs::prelude::Resource;
use bevy_ecs::schedule::{InternedScheduleLabel, IntoScheduleConfigs, SystemSet};
use bevy_ecs::world::World;
use calloop::EventLoop;
use std::collections::HashMap;

use crate::calloop::{
    CalloopSourceRegistry, WaylandCalloopRuntime, dispatch_wayland_calloop_system,
};
use crate::error::NekolandError;
use crate::lifecycle::AppLifecycleState;
use crate::plugin::{NekolandAppPlugin, NekolandPlugin};
use crate::schedules::{
    ExtractSchedule, InputSchedule, LayoutSchedule, PostRenderSchedule, PreRenderSchedule,
    PresentSchedule, ProtocolSchedule, RenderSchedule, install_core_schedules,
    install_core_schedules_sub_app,
};

/// Bevy sub-app label for the platform-facing Wayland/backend world.
#[derive(AppLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WaylandSubApp;

/// Bevy sub-app label for the render-world extraction and compilation pipeline.
#[derive(AppLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderSubApp;

/// Extract-phase set that polls the platform runtime event loop.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandPollSystems;

/// Extract-phase set that pulls runtime state into frame-local platform queues and snapshots.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandExtractSystems;

/// Set that normalizes raw platform/runtime state into boundary-friendly snapshots.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandNormalizeSystems;

/// Set that applies shell-driven commands inside the platform sub-app.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandApplySystems;

/// Present-phase set that performs backend/protocol-side submission work.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandPresentSystems;

/// Set that emits platform feedback after normalize/apply or present work completes.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandFeedbackSystems;

/// Final present-phase set that clears frame-local platform queues.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaylandCleanupSystems;

/// Lightweight metadata describing the running compositor process.
#[derive(Debug, Clone, Resource)]
pub struct AppMetadata {
    /// Human-readable application name used in logging and backend-facing labels.
    pub name: String,
}

type SubAppSyncBackFn = Box<dyn FnMut(&mut World, &mut World, Option<InternedScheduleLabel>)>;

#[derive(Default)]
struct SubAppSyncRegistry {
    hooks: HashMap<InternedAppLabel, SubAppSyncBackFn>,
}

/// Runtime knobs for the outer frame loop.
///
/// Both fields are intentionally overridable via environment variables so integration tests can
/// cap frame counts and shorten waits without patching application code.
#[derive(Debug, Clone, Resource)]
pub struct RunLoopSettings {
    /// Maximum time the outer frame loop waits while polling the platform event loop.
    pub frame_timeout: Duration,
    /// Optional frame cap used mainly by tests and deterministic short-lived runs.
    pub max_frames: Option<u64>,
}

impl Default for RunLoopSettings {
    fn default() -> Self {
        let max_frames = match std::env::var("NEKOLAND_MAX_FRAMES") {
            Ok(value) => value
                .parse::<u64>()
                .ok()
                .map(|value| if value == 0 { None } else { Some(value) })
                .unwrap_or(None),
            Err(_) => None,
        };
        let frame_timeout = std::env::var("NEKOLAND_FRAME_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(16));

        Self { frame_timeout, max_frames }
    }
}

/// Root application wrapper that owns the main world plus the Wayland and render sub-apps.
pub struct NekolandApp {
    app: App,
}

impl NekolandApp {
    /// Creates a new compositor app with the canonical schedules and empty sub-app shells.
    pub fn new(name: impl Into<String>) -> Self {
        let mut app = App::new();
        app.set_error_handler(warn);
        app.insert_resource(AppMetadata { name: name.into() });
        app.insert_resource(AppLifecycleState::default());
        app.insert_resource(RunLoopSettings::default());
        app.insert_non_send_resource(SubAppSyncRegistry::default());
        install_core_schedules(&mut app);
        app.insert_sub_app(WaylandSubApp, empty_nekoland_sub_app());
        app.insert_sub_app(RenderSubApp, empty_nekoland_sub_app());
        app.sub_app_mut(WaylandSubApp)
            .configure_sets(
                ExtractSchedule,
                (
                    WaylandPollSystems,
                    WaylandExtractSystems.after(WaylandPollSystems),
                    WaylandNormalizeSystems.after(WaylandExtractSystems),
                    WaylandApplySystems.after(WaylandNormalizeSystems),
                ),
            )
            .configure_sets(
                ProtocolSchedule,
                (
                    WaylandNormalizeSystems,
                    WaylandApplySystems.after(WaylandNormalizeSystems),
                    WaylandFeedbackSystems.after(WaylandApplySystems),
                ),
            )
            .configure_sets(
                PresentSchedule,
                (
                    WaylandPresentSystems,
                    WaylandFeedbackSystems.after(WaylandPresentSystems),
                    WaylandCleanupSystems.after(WaylandFeedbackSystems),
                ),
            )
            .add_systems(
                ExtractSchedule,
                dispatch_wayland_calloop_system.in_set(WaylandPollSystems),
            );

        Self { app }
    }

    /// Registers one plugin on the main shell world.
    ///
    /// This is where shell policy, config, IPC, and other main-world systems are installed.
    pub fn add_plugin<P>(&mut self, plugin: P) -> &mut Self
    where
        P: NekolandPlugin,
    {
        tracing::debug!(plugin = plugin.name(), "registering nekoland plugin");
        plugin.build(&mut self.app);
        self
    }

    /// Registers one plugin on the Wayland/platform sub-app.
    pub fn add_wayland_plugin<P>(&mut self, plugin: P) -> &mut Self
    where
        P: NekolandPlugin,
    {
        tracing::debug!(plugin = plugin.name(), "registering nekoland wayland sub-app plugin");
        self.app.sub_app_mut(WaylandSubApp).add_plugins(NekolandAppPlugin::new(plugin));
        self
    }

    /// Registers one plugin on the render sub-app.
    pub fn add_render_plugin<P>(&mut self, plugin: P) -> &mut Self
    where
        P: NekolandPlugin,
    {
        tracing::debug!(plugin = plugin.name(), "registering nekoland render sub-app plugin");
        self.app.sub_app_mut(RenderSubApp).add_plugins(NekolandAppPlugin::new(plugin));
        self
    }

    /// Installs a sync-back hook that mirrors selected sub-app resources into the main world.
    pub fn set_sub_app_sync_back(
        &mut self,
        label: impl AppLabel,
        sync: impl FnMut(&mut World, &mut World, Option<InternedScheduleLabel>) + 'static,
    ) -> &mut Self {
        let mut registry = self
            .app
            .world_mut()
            .get_non_send_resource_mut::<SubAppSyncRegistry>()
            .expect("sub-app sync registry should be installed");
        registry.hooks.insert(label.intern(), Box::new(sync));
        self
    }

    /// Inserts one resource into the main world.
    pub fn insert_resource<R>(&mut self, resource: R) -> &mut Self
    where
        R: Resource,
    {
        self.app.insert_resource(resource);
        self
    }

    /// Returns the underlying Bevy app.
    pub fn inner(&self) -> &App {
        &self.app
    }

    /// Returns the underlying Bevy app mutably.
    pub fn inner_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// Runs the compositor until shutdown is requested or the configured frame cap is reached.
    pub fn run(&mut self) -> Result<(), NekolandError> {
        let settings =
            self.app.world().get_resource::<RunLoopSettings>().cloned().unwrap_or_default();
        let event_loop: EventLoop<()> =
            EventLoop::try_new().map_err(|error| NekolandError::Runtime(error.to_string()))?;
        if let Some(mut registry) =
            self.app.world_mut().get_non_send_resource_mut::<CalloopSourceRegistry>()
        {
            registry.install_all(&event_loop.handle())?;
        }
        if let Some(mut registry) = self
            .app
            .sub_app_mut(WaylandSubApp)
            .world_mut()
            .get_non_send_resource_mut::<CalloopSourceRegistry>()
        {
            registry.install_all(&event_loop.handle())?;
        }
        let started_at = Instant::now();
        let mut frame = 0_u64;

        tracing::info!(
            ?settings.max_frames,
            frame_timeout_ms = settings.frame_timeout.as_millis(),
            "starting nekoland run loop"
        );
        self.app.sub_app_mut(WaylandSubApp).world_mut().insert_resource(settings.clone());
        self.app
            .sub_app_mut(WaylandSubApp)
            .world_mut()
            .insert_non_send_resource(WaylandCalloopRuntime::new(event_loop));

        while settings.max_frames.map(|max| frame < max).unwrap_or(true) {
            self.run_wayland_extract_phase();
            self.run_main_shell_phase();
            self.run_render_phase();
            self.run_wayland_present_phase();

            if self
                .app
                .world()
                .get_resource::<AppLifecycleState>()
                .is_some_and(|state| state.quit_requested)
            {
                tracing::info!(frame, "stopping nekoland run loop after quit request");
                break;
            }

            frame += 1;
            tracing::trace!(frame, uptime_ms = started_at.elapsed().as_millis(), "frame complete");
        }

        Ok(())
    }

    fn run_wayland_extract_phase(&mut self) {
        // First fan out shell-owned wayland commands into platform-owned pending queues.
        self.update_sub_app_with_sync_schedule(WaylandSubApp, ExtractSchedule);

        // Then run the main-world platform extraction path for the current frame.
        self.app.main_mut().update();
        let world = self.app.world_mut();
        world.run_schedule(ExtractSchedule);
        world.run_schedule(ProtocolSchedule);

        // Finally mirror the normalized platform snapshots back into WaylandIngress.
        self.update_sub_app_with_sync_schedule(WaylandSubApp, ProtocolSchedule);
    }

    fn run_main_shell_phase(&mut self) {
        let world = self.app.world_mut();
        world.run_schedule(InputSchedule);
        world.run_schedule(LayoutSchedule);
    }

    fn run_render_phase(&mut self) {
        self.app.world_mut().run_schedule(PreRenderSchedule);
        self.app.world_mut().run_schedule(RenderSchedule);
        self.update_sub_app_with_sync(RenderSubApp);
        self.app.world_mut().run_schedule(PostRenderSchedule);
    }

    fn run_wayland_present_phase(&mut self) {
        self.app.world_mut().run_schedule(PresentSchedule);
        self.update_sub_app_with_sync_schedule(WaylandSubApp, PresentSchedule);
    }

    fn update_sub_app_with_sync(&mut self, label: impl AppLabel + Copy) {
        self.update_sub_app_with_sync_schedule_internal(label, None);
    }

    fn update_sub_app_with_sync_schedule(
        &mut self,
        label: impl AppLabel + Copy,
        schedule: impl bevy_ecs::schedule::ScheduleLabel,
    ) {
        self.update_sub_app_with_sync_schedule_internal(label, Some(schedule.intern()));
    }

    fn update_sub_app_with_sync_schedule_internal(
        &mut self,
        label: impl AppLabel + Copy,
        schedule: Option<bevy_ecs::schedule::InternedScheduleLabel>,
    ) {
        let interned = label.intern();
        let Some(mut sub_app) = self.app.remove_sub_app(label) else {
            return;
        };
        if let Some(schedule) = schedule {
            sub_app.update_schedule = Some(schedule);
        }
        sub_app.extract(self.app.world_mut());
        sub_app.update();

        let sync = self
            .app
            .world_mut()
            .get_non_send_resource_mut::<SubAppSyncRegistry>()
            .and_then(|mut registry| registry.hooks.remove(&interned));
        if let Some(mut sync) = sync {
            sync(self.app.world_mut(), sub_app.world_mut(), schedule);
            if let Some(mut registry) =
                self.app.world_mut().get_non_send_resource_mut::<SubAppSyncRegistry>()
            {
                registry.hooks.insert(interned, sync);
            }
        }

        self.app.insert_sub_app(label, sub_app);
    }
}

fn empty_nekoland_sub_app() -> SubApp {
    let mut app = SubApp::new();
    install_core_schedules_sub_app(&mut app);
    app
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use bevy_app::Update;
    use bevy_ecs::prelude::Resource;
    use bevy_ecs::schedule::{IntoScheduleConfigs, ScheduleLabel};
    use calloop::timer::{TimeoutAction, Timer};

    use crate::plugin::NekolandPlugin;
    use crate::schedules::{ExtractSchedule, LayoutSchedule, PresentSchedule, ProtocolSchedule};
    use crate::{
        calloop::{CalloopSourceRegistry, WaylandCalloopRuntime},
        error::NekolandError,
    };

    use bevy_app::App;

    use super::{NekolandApp, RenderSubApp, RunLoopSettings, WaylandPollSystems, WaylandSubApp};

    #[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
    struct TestMarker(u8);

    #[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct TickCounter(u32);

    #[derive(Resource, Debug, Default, PartialEq, Eq)]
    struct TraceLog(Vec<&'static str>);

    #[derive(Resource, Debug, Default, PartialEq, Eq)]
    struct SubTraceLog(Vec<&'static str>);

    #[derive(Debug, Clone, Copy)]
    struct MarkerPlugin {
        marker: u8,
    }

    impl NekolandPlugin for MarkerPlugin {
        fn build(&self, app: &mut App) {
            app.insert_resource(TestMarker(self.marker));
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct TickPlugin;

    impl NekolandPlugin for TickPlugin {
        fn build(&self, app: &mut App) {
            app.init_resource::<TickCounter>()
                .add_systems(
                    ExtractSchedule,
                    (|mut counter: bevy_ecs::prelude::ResMut<'_, TickCounter>| {
                        counter.0 = counter.0.saturating_add(1);
                    },)
                        .chain(),
                )
                .add_systems(
                    ProtocolSchedule,
                    (|mut counter: bevy_ecs::prelude::ResMut<'_, TickCounter>| {
                        counter.0 = counter.0.saturating_add(1);
                    },)
                        .chain(),
                )
                .add_systems(
                    PresentSchedule,
                    (|mut counter: bevy_ecs::prelude::ResMut<'_, TickCounter>| {
                        counter.0 = counter.0.saturating_add(1);
                    },)
                        .chain(),
                );
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct MainTracePlugin;

    impl NekolandPlugin for MainTracePlugin {
        fn build(&self, app: &mut App) {
            app.init_resource::<TraceLog>().add_systems(
                LayoutSchedule,
                (|mut trace: bevy_ecs::prelude::ResMut<'_, TraceLog>| {
                    trace.0.push("main");
                },)
                    .chain(),
            );
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct WaylandTracePlugin;

    impl NekolandPlugin for WaylandTracePlugin {
        fn build(&self, app: &mut App) {
            app.init_resource::<SubTraceLog>()
                .add_systems(
                    ExtractSchedule,
                    (|mut trace: bevy_ecs::prelude::ResMut<'_, SubTraceLog>| {
                        trace.0.push("wayland_extract");
                    },)
                        .chain(),
                )
                .add_systems(
                    PresentSchedule,
                    (|mut trace: bevy_ecs::prelude::ResMut<'_, SubTraceLog>| {
                        trace.0.push("wayland_present");
                    },)
                        .chain(),
                );
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct RenderTracePlugin;

    impl NekolandPlugin for RenderTracePlugin {
        fn build(&self, app: &mut App) {
            app.init_resource::<SubTraceLog>().add_systems(
                Update,
                (|mut trace: bevy_ecs::prelude::ResMut<'_, SubTraceLog>| {
                    trace.0.push("render");
                },)
                    .chain(),
            );
        }
    }

    #[derive(Debug, Default, Resource, PartialEq, Eq)]
    struct PollAudit(u32);

    #[derive(Debug, Clone)]
    struct CalloopPollTracePlugin {
        shared: Arc<AtomicU32>,
    }

    impl NekolandPlugin for CalloopPollTracePlugin {
        fn build(&self, app: &mut App) {
            let shared = self.shared.clone();
            app.init_resource::<PollAudit>().add_systems(
                ExtractSchedule,
                (move |mut audit: bevy_ecs::prelude::ResMut<'_, PollAudit>| {
                    audit.0 = shared.load(Ordering::SeqCst);
                },)
                    .chain()
                    .after(WaylandPollSystems),
            );
        }
    }

    #[test]
    fn new_app_installs_default_warn_error_handler() {
        let app = NekolandApp::new("error-handler-test");

        assert!(
            app.inner().get_error_handler().is_some(),
            "NekolandApp::new should install a default Bevy ECS error handler",
        );
    }

    #[test]
    fn raw_bevy_app_starts_without_nekoland_error_handler() {
        let app = App::new();

        assert!(
            app.get_error_handler().is_none(),
            "control check: plain App::new should not carry the nekoland default handler",
        );
    }

    #[test]
    fn new_app_installs_wayland_and_render_subapps() {
        let app = NekolandApp::new("sub-app-install-test");

        assert!(app.inner().get_sub_app(WaylandSubApp).is_some());
        assert!(app.inner().get_sub_app(RenderSubApp).is_some());
    }

    #[test]
    fn subapp_plugin_registration_targets_the_requested_subapp() {
        let mut app = NekolandApp::new("sub-app-plugin-test");
        app.add_wayland_plugin(MarkerPlugin { marker: 1 });
        app.add_render_plugin(MarkerPlugin { marker: 2 });

        let wayland = app.inner().sub_app(WaylandSubApp);
        let render = app.inner().sub_app(RenderSubApp);

        assert_eq!(wayland.world().get_resource::<TestMarker>(), Some(&TestMarker(1)));
        assert_eq!(render.world().get_resource::<TestMarker>(), Some(&TestMarker(2)));
        assert!(app.inner().world().get_resource::<TestMarker>().is_none());
    }

    #[test]
    fn wayland_subapp_runs_during_extract_and_present_phases() {
        let mut app = NekolandApp::new("wayland-sub-app-run-test");
        app.add_wayland_plugin(TickPlugin);

        app.run_wayland_extract_phase();
        app.run_wayland_present_phase();

        let wayland = app.inner().sub_app(WaylandSubApp);
        assert_eq!(wayland.world().get_resource::<TickCounter>(), Some(&TickCounter(3)));
    }

    #[test]
    fn wayland_subapp_extract_phase_polls_calloop_runtime() {
        let mut app = NekolandApp::new("wayland-sub-app-poll-test");
        let shared = Arc::new(AtomicU32::new(0));
        app.add_wayland_plugin(CalloopPollTracePlugin { shared: shared.clone() });

        {
            let wayland_world = app.inner_mut().sub_app_mut(WaylandSubApp).world_mut();
            wayland_world.insert_resource(RunLoopSettings {
                frame_timeout: Duration::from_millis(0),
                max_frames: Some(1),
            });
            wayland_world.insert_non_send_resource(CalloopSourceRegistry::default());
            let mut registry = wayland_world
                .get_non_send_resource_mut::<CalloopSourceRegistry>()
                .expect("wayland calloop registry should exist");
            let shared_for_timer = shared.clone();
            registry.push(move |handle| {
                let timer_shared = shared_for_timer.clone();
                handle
                    .insert_source(Timer::immediate(), move |_, _, _| {
                        timer_shared.fetch_add(1, Ordering::SeqCst);
                        TimeoutAction::Drop
                    })
                    .map_err(|error| NekolandError::Runtime(error.error.to_string()))?;
                Ok(())
            });
        }

        let event_loop = calloop::EventLoop::try_new().expect("test event loop should init");
        {
            let wayland_world = app.inner_mut().sub_app_mut(WaylandSubApp).world_mut();
            let mut registry = wayland_world
                .get_non_send_resource_mut::<CalloopSourceRegistry>()
                .expect("wayland calloop registry should exist");
            registry
                .install_all(&event_loop.handle())
                .expect("calloop registry should install into test loop");
            wayland_world.insert_non_send_resource(WaylandCalloopRuntime::new(event_loop));
        }

        app.run_wayland_extract_phase();

        let wayland = app.inner().sub_app(WaylandSubApp);
        assert_eq!(shared.load(Ordering::SeqCst), 1);
        assert_eq!(wayland.world().get_resource::<PollAudit>(), Some(&PollAudit(1)));
    }

    #[test]
    fn runner_smoke_path_preserves_wayland_main_render_wayland_order() {
        let mut app = NekolandApp::new("runner-smoke-path-test");
        app.add_plugin(MainTracePlugin)
            .add_wayland_plugin(WaylandTracePlugin)
            .add_render_plugin(RenderTracePlugin)
            .set_sub_app_sync_back(WaylandSubApp, |main_world, sub_world, _schedule| {
                let entries = sub_world
                    .get_resource_mut::<SubTraceLog>()
                    .map(|mut trace| std::mem::take(&mut trace.0))
                    .unwrap_or_default();
                if entries.is_empty() {
                    return;
                }
                let mut main_trace = main_world.resource_mut::<TraceLog>();
                main_trace.0.extend(entries);
            })
            .set_sub_app_sync_back(RenderSubApp, |main_world, sub_world, _schedule| {
                let entries = sub_world
                    .get_resource_mut::<SubTraceLog>()
                    .map(|mut trace| std::mem::take(&mut trace.0))
                    .unwrap_or_default();
                if entries.is_empty() {
                    return;
                }
                let mut main_trace = main_world.resource_mut::<TraceLog>();
                main_trace.0.extend(entries);
            });
        app.inner_mut().sub_app_mut(RenderSubApp).update_schedule = Some(Update.intern());

        app.run_wayland_extract_phase();
        app.run_main_shell_phase();
        app.run_render_phase();
        app.run_wayland_present_phase();

        assert_eq!(
            app.inner().world().resource::<TraceLog>().0,
            vec!["wayland_extract", "main", "render", "wayland_present"]
        );
    }
}
