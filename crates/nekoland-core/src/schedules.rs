use bevy_app::App;
use bevy_ecs::schedule::ScheduleLabel;

/// Frame phase that extracts external runtime state into ECS resources.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExtractSchedule;

/// Frame phase that flushes protocol callbacks into ECS-owned request queues.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProtocolSchedule;

/// Frame phase that translates backend input into higher-level ECS events and requests.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputSchedule;

/// Frame phase that applies shell/layout/focus state transitions.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutSchedule;

/// Frame phase that composes render data, pacing state, and render-side effects.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderSchedule;

/// Final frame phase that submits or presents the rendered frame.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PresentSchedule;

/// Registers the canonical nekoland frame schedules onto the app.
pub fn install_core_schedules(app: &mut App) {
    app.init_schedule(ExtractSchedule)
        .init_schedule(ProtocolSchedule)
        .init_schedule(InputSchedule)
        .init_schedule(LayoutSchedule)
        .init_schedule(RenderSchedule)
        .init_schedule(PresentSchedule);
}
