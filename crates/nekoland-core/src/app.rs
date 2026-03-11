use std::time::{Duration, Instant};

use bevy_app::App;
use bevy_ecs::prelude::Resource;
use calloop::EventLoop;

use crate::calloop::CalloopSourceRegistry;
use crate::error::NekolandError;
use crate::plugin::NekolandPlugin;
use crate::schedules::{
    ExtractSchedule, InputSchedule, LayoutSchedule, PresentSchedule, ProtocolSchedule,
    RenderSchedule, install_core_schedules,
};

#[derive(Debug, Clone, Resource)]
pub struct AppMetadata {
    pub name: String,
}

#[derive(Debug, Clone, Resource)]
pub struct RunLoopSettings {
    pub frame_timeout: Duration,
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

pub struct NekolandApp {
    app: App,
}

impl NekolandApp {
    pub fn new(name: impl Into<String>) -> Self {
        let mut app = App::new();
        app.insert_resource(AppMetadata { name: name.into() });
        app.insert_resource(RunLoopSettings::default());
        install_core_schedules(&mut app);

        Self { app }
    }

    pub fn add_plugin<P>(&mut self, plugin: P) -> &mut Self
    where
        P: NekolandPlugin,
    {
        tracing::debug!(plugin = plugin.name(), "registering nekoland plugin");
        plugin.build(&mut self.app);
        self
    }

    pub fn insert_resource<R>(&mut self, resource: R) -> &mut Self
    where
        R: Resource,
    {
        self.app.insert_resource(resource);
        self
    }

    pub fn inner(&self) -> &App {
        &self.app
    }

    pub fn inner_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn run(&mut self) -> Result<(), NekolandError> {
        let settings =
            self.app.world().get_resource::<RunLoopSettings>().cloned().unwrap_or_default();
        let mut event_loop: EventLoop<()> =
            EventLoop::try_new().map_err(|error| NekolandError::Runtime(error.to_string()))?;
        if let Some(mut registry) =
            self.app.world_mut().get_non_send_resource_mut::<CalloopSourceRegistry>()
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

        while settings.max_frames.map(|max| frame < max).unwrap_or(true) {
            event_loop
                .dispatch(settings.frame_timeout, &mut ())
                .map_err(|error| NekolandError::Runtime(error.to_string()))?;

            self.app.update();
            self.app.world_mut().run_schedule(ExtractSchedule);
            self.app.world_mut().run_schedule(ProtocolSchedule);
            self.app.world_mut().run_schedule(InputSchedule);
            self.app.world_mut().run_schedule(LayoutSchedule);
            self.app.world_mut().run_schedule(RenderSchedule);
            self.app.world_mut().run_schedule(PresentSchedule);

            frame += 1;
            tracing::trace!(frame, uptime_ms = started_at.elapsed().as_millis(), "frame complete");
        }

        Ok(())
    }
}
