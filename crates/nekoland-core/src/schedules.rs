use bevy_app::App;
use bevy_ecs::schedule::ScheduleLabel;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExtractSchedule;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProtocolSchedule;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputSchedule;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutSchedule;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderSchedule;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PresentSchedule;

pub fn install_core_schedules(app: &mut App) {
    app.init_schedule(ExtractSchedule)
        .init_schedule(ProtocolSchedule)
        .init_schedule(InputSchedule)
        .init_schedule(LayoutSchedule)
        .init_schedule(RenderSchedule)
        .init_schedule(PresentSchedule);
}
