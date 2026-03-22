#[derive(Debug, Default)]
pub(crate) struct WorkspaceVisibilityState {
    pub(crate) initialized: bool,
    pub(crate) active_workspace: Option<u32>,
    pub(crate) visible_toplevels: std::collections::BTreeSet<u64>,
    pub(crate) visible_popups: std::collections::BTreeSet<u64>,
    pub(crate) hidden_parent_popups: std::collections::BTreeSet<u64>,
}

#[derive(Debug, Clone, Default, bevy_ecs::prelude::Resource)]
pub(crate) struct WorkspaceVisibilitySnapshot {
    pub active_workspace: Option<u32>,
    pub visible_toplevels: std::collections::BTreeSet<u64>,
    pub visible_popups: std::collections::BTreeSet<u64>,
    pub hidden_parent_popups: std::collections::BTreeSet<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputTiming {
    pub(crate) output_name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) refresh_millihz: u32,
    pub(crate) scale: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PresentationFeedbackTiming {
    pub(crate) frame_time: super::Time<super::Monotonic>,
    pub(crate) refresh: super::Refresh,
    pub(crate) sequence: Option<u64>,
}

pub(crate) fn dispatch_surface_frame_callbacks_system(
    output_snapshots: Option<
        bevy_ecs::prelude::Res<'_, nekoland_ecs::resources::OutputSnapshotState>,
    >,
    output_presentation: Option<
        bevy_ecs::prelude::Res<'_, nekoland_ecs::resources::OutputPresentationState>,
    >,
    frame_pacing: bevy_ecs::prelude::Res<'_, nekoland_ecs::resources::FramePacingState>,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    if frame_pacing.callback_surface_ids.is_empty()
        && frame_pacing.presentation_surface_ids.is_empty()
    {
        return;
    }

    let timing =
        current_output_presentation(output_snapshots.as_deref(), output_presentation.as_deref())
            .unwrap_or_else(|| {
                let frame_time = super::Clock::<super::Monotonic>::new().now();
                let refresh = current_output_timing(output_snapshots.as_deref())
                    .map(refresh_from_output_timing)
                    .unwrap_or(super::Refresh::Unknown);
                PresentationFeedbackTiming { frame_time, refresh, sequence: None }
            });
    server.send_frame_callbacks(&frame_pacing.callback_surface_ids, timing.frame_time);
    server.send_presentation_feedback(
        &frame_pacing.presentation_surface_ids,
        timing.frame_time,
        timing.refresh,
        timing.sequence,
    );
}

pub(crate) fn sync_workspace_visibility_system(
    visibility_snapshot: bevy_ecs::prelude::Res<'_, WorkspaceVisibilitySnapshot>,
    mut visibility: bevy_ecs::prelude::Local<'_, WorkspaceVisibilityState>,
    server: Option<bevy_ecs::prelude::NonSendMut<'_, super::server::SmithayProtocolServer>>,
) {
    let Some(mut server) = server else {
        return;
    };
    let active_workspace = visibility_snapshot.active_workspace;
    let visible_toplevels = visibility_snapshot.visible_toplevels.clone();
    let visible_popups = visibility_snapshot.visible_popups.clone();
    let hidden_parent_popups = visibility_snapshot.hidden_parent_popups.clone();

    if !visibility.initialized {
        visibility.initialized = true;
        visibility.active_workspace = active_workspace;
        visibility.visible_toplevels = visible_toplevels;
        visibility.visible_popups = visible_popups;
        visibility.hidden_parent_popups = hidden_parent_popups;
        return;
    }

    let dismissed_popups = visibility
        .visible_popups
        .difference(&visible_popups)
        .copied()
        .chain(hidden_parent_popups.difference(&visibility.hidden_parent_popups).copied())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let activated_toplevels =
        visible_toplevels.difference(&visibility.visible_toplevels).copied().collect::<Vec<_>>();

    if visibility.active_workspace != active_workspace
        || visibility.visible_toplevels != visible_toplevels
        || visibility.visible_popups != visible_popups
        || visibility.hidden_parent_popups != hidden_parent_popups
    {
        server.sync_workspace_visibility(&activated_toplevels, &dismissed_popups);
    }

    visibility.active_workspace = active_workspace;
    visibility.visible_toplevels = visible_toplevels;
    visibility.visible_popups = visible_popups;
    visibility.hidden_parent_popups = hidden_parent_popups;
}

pub(crate) fn current_output_timing(
    output_snapshots: Option<&nekoland_ecs::resources::OutputSnapshotState>,
) -> Option<OutputTiming> {
    output_snapshots?.outputs.iter().min_by(|left, right| left.name.cmp(&right.name)).map(
        |output| OutputTiming {
            output_name: output.name.clone(),
            width: output.width.max(1),
            height: output.height.max(1),
            refresh_millihz: output.refresh_millihz,
            scale: output.scale.max(1),
        },
    )
}

pub(crate) fn current_output_presentation(
    output_snapshots: Option<&nekoland_ecs::resources::OutputSnapshotState>,
    output_presentation: Option<&nekoland_ecs::resources::OutputPresentationState>,
) -> Option<PresentationFeedbackTiming> {
    let output_presentation = output_presentation?;
    let output_id = output_snapshots?
        .outputs
        .iter()
        .min_by(|left, right| left.name.cmp(&right.name))
        .map(|output| output.output_id)?;
    let timeline =
        output_presentation.outputs.iter().find(|timeline| timeline.output_id == output_id)?;
    let frame_time = super::Time::<super::Monotonic>::from(std::time::Duration::from_nanos(
        timeline.present_time_nanos,
    ));
    let refresh = if timeline.refresh_interval_nanos == 0 {
        super::Refresh::Unknown
    } else {
        super::Refresh::fixed(std::time::Duration::from_nanos(timeline.refresh_interval_nanos))
    };

    Some(PresentationFeedbackTiming { frame_time, refresh, sequence: Some(timeline.sequence) })
}

pub(crate) fn refresh_from_output_timing(output_timing: OutputTiming) -> super::Refresh {
    if output_timing.refresh_millihz == 0 {
        return super::Refresh::Unknown;
    }

    let refresh_nanos = 1_000_000_000_000_u64 / u64::from(output_timing.refresh_millihz);
    super::Refresh::fixed(std::time::Duration::from_nanos(refresh_nanos.max(1)))
}
