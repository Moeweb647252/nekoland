use bevy_ecs::prelude::{Commands, Entity};
use nekoland_ecs::components::{
    OutputBackgroundWindow, WindowFullscreenTarget, WindowLayout, WindowMode, WindowPolicy,
    WindowPolicyState, WindowRestoreSnapshot, WindowRole, WindowSceneGeometry,
};
use nekoland_ecs::components::{OutputId, WindowRestoreState};
use nekoland_ecs::selectors::OutputName;
use nekoland_ecs::views::OutputRuntime;

/// Applies a newly resolved default policy to a window and marks policy updates as unlocked.
pub fn apply_window_policy(
    policy: WindowPolicy,
    layout: &mut WindowLayout,
    mode: &mut WindowMode,
    policy_state: &mut WindowPolicyState,
) {
    policy.apply(layout, mode);
    policy_state.applied = policy;
    policy_state.locked = false;
}

/// Refreshes a window's default policy after metadata changes.
///
/// The visible layout/mode are only updated while the window is still tracking the previously
/// applied policy. Temporary restore snapshots are updated when they still point at that same
/// policy so unmaximize/unfullscreen flows converge to the refreshed default.
pub fn refresh_window_policy(
    policy: WindowPolicy,
    layout: &mut WindowLayout,
    mode: &mut WindowMode,
    restore: &mut WindowRestoreSnapshot,
    policy_state: &mut WindowPolicyState,
) {
    if policy_state.locked {
        return;
    }

    let previous = policy_state.applied;
    if policy_state.tracks_current(*layout, *mode) {
        policy.apply(layout, mode);
    }

    if let Some(snapshot) = restore.snapshot.as_mut() {
        refresh_restore_snapshot_policy(snapshot, previous, policy);
    }

    policy_state.applied = policy;
}

/// Prevents later metadata changes from rewriting the window's base layout/mode semantics.
pub fn lock_window_policy(
    layout: WindowLayout,
    mode: WindowMode,
    policy_state: &mut WindowPolicyState,
) {
    policy_state.applied = WindowPolicy::new(layout, mode);
    policy_state.locked = true;
}

/// Restores the current layout/mode back to the stored default policy.
pub fn restore_window_policy(
    policy_state: &WindowPolicyState,
    layout: &mut WindowLayout,
    mode: &mut WindowMode,
) {
    policy_state.applied.apply(layout, mode);
}

/// Pushes the current window state onto the restore stack before entering a temporary mode.
///
/// Re-entering the same temporary mode is treated as idempotent so repeated maximize/fullscreen
/// requests do not clobber the original restore point.
pub fn enter_temporary_window_mode(
    scene_geometry: &WindowSceneGeometry,
    fullscreen_target: &mut WindowFullscreenTarget,
    restore: &mut WindowRestoreSnapshot,
    layout: WindowLayout,
    mode: &mut WindowMode,
    target_mode: WindowMode,
    target_fullscreen_output: Option<OutputName>,
) {
    if *mode != target_mode {
        restore.snapshot = Some(WindowRestoreState {
            geometry: scene_geometry.clone(),
            layout,
            mode: *mode,
            fullscreen_output: fullscreen_target.output.clone(),
            previous: restore.snapshot.take().map(Box::new),
        });
    }

    *mode = target_mode;
    fullscreen_target.output = target_fullscreen_output;
}

/// Restores one temporary-mode frame, falling back to the stored default policy when no snapshot
/// remains.
pub fn restore_window_state(
    policy_state: &WindowPolicyState,
    scene_geometry: &mut WindowSceneGeometry,
    fullscreen_target: &mut WindowFullscreenTarget,
    restore: &mut WindowRestoreSnapshot,
    layout: &mut WindowLayout,
    mode: &mut WindowMode,
) {
    if let Some(restored) = restore.snapshot.take() {
        let WindowRestoreState {
            geometry,
            layout: restored_layout,
            mode: restored_mode,
            fullscreen_output,
            previous,
        } = restored;
        *scene_geometry = geometry;
        fullscreen_target.output = fullscreen_output;
        *layout = restored_layout;
        *mode = restored_mode;
        restore.snapshot = previous.map(|previous| *previous);
    } else {
        restore_window_policy(policy_state, layout, mode);
        fullscreen_target.output = None;
    }
}

fn refresh_restore_snapshot_policy(
    snapshot: &mut WindowRestoreState,
    previous: WindowPolicy,
    policy: WindowPolicy,
) {
    if snapshot.layout == previous.layout && snapshot.mode == previous.mode {
        snapshot.layout = policy.layout;
        snapshot.mode = policy.mode;
    }

    if let Some(earlier) = snapshot.previous.as_mut() {
        refresh_restore_snapshot_policy(earlier, previous, policy);
    }
}

pub struct WindowBackgroundState<'a> {
    pub role: &'a mut WindowRole,
    pub scene_geometry: &'a mut WindowSceneGeometry,
    pub fullscreen_target: &'a mut WindowFullscreenTarget,
    pub layout: &'a mut WindowLayout,
    pub mode: &'a mut WindowMode,
}

impl<'a> WindowBackgroundState<'a> {
    pub fn new(
        role: &'a mut WindowRole,
        scene_geometry: &'a mut WindowSceneGeometry,
        fullscreen_target: &'a mut WindowFullscreenTarget,
        layout: &'a mut WindowLayout,
        mode: &'a mut WindowMode,
    ) -> Self {
        Self { role, scene_geometry, fullscreen_target, layout, mode }
    }
}

pub fn resolve_background_output_id<'w, 's>(
    outputs: &bevy_ecs::prelude::Query<'w, 's, (Entity, OutputRuntime)>,
    desired_output: Option<&OutputName>,
) -> Option<OutputId> {
    let output_name = desired_output.map(OutputName::as_str)?;
    outputs.iter().find(|(_, output)| output.name() == output_name).map(|(_, output)| output.id())
}

pub fn sync_window_background_role(
    commands: &mut Commands,
    entity: Entity,
    desired_output: Option<OutputId>,
    window: WindowBackgroundState<'_>,
    current_background: Option<OutputBackgroundWindow>,
) {
    let WindowBackgroundState { role, scene_geometry, fullscreen_target, layout, mode } = window;
    let current_output = current_background.as_ref().map(|background| background.output.clone());

    if desired_output == current_output {
        return;
    }

    match desired_output {
        Some(output) => {
            let restore = current_background.map(|background| background.restore).unwrap_or(
                WindowRestoreState {
                    geometry: scene_geometry.clone(),
                    layout: *layout,
                    mode: *mode,
                    fullscreen_output: fullscreen_target.output.clone(),
                    previous: None,
                },
            );
            *mode = WindowMode::Fullscreen;
            *role = WindowRole::OutputBackground;
            commands.entity(entity).insert(OutputBackgroundWindow { output, restore });
        }
        None => {
            if let Some(background) = current_background {
                *scene_geometry = background.restore.geometry.clone();
                fullscreen_target.output = background.restore.fullscreen_output.clone();
                *layout = background.restore.layout;
                *mode = background.restore.mode;
                *role = WindowRole::Managed;
                commands.entity(entity).remove::<OutputBackgroundWindow>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::prelude::World;
    use nekoland_ecs::components::{
        OutputBackgroundWindow, OutputId, WindowFullscreenTarget, WindowRestoreState, WindowRole,
        WindowSceneGeometry,
    };
    use nekoland_ecs::selectors::OutputName;

    use super::{
        WindowBackgroundState, WindowLayout, WindowMode, WindowPolicy, WindowPolicyState,
        WindowRestoreSnapshot, apply_window_policy, enter_temporary_window_mode,
        lock_window_policy, refresh_window_policy, restore_window_state,
        sync_window_background_role,
    };

    #[test]
    fn refresh_updates_current_policy_when_window_is_still_tracking_defaults() {
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Normal;
        let mut restore = WindowRestoreSnapshot::default();
        let mut policy_state = WindowPolicyState::default();

        apply_window_policy(
            WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal),
            &mut layout,
            &mut mode,
            &mut policy_state,
        );
        refresh_window_policy(
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal),
            &mut layout,
            &mut mode,
            &mut restore,
            &mut policy_state,
        );

        assert_eq!(layout, WindowLayout::Tiled);
        assert_eq!(mode, WindowMode::Normal);
        assert_eq!(
            policy_state.applied,
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal)
        );
    }

    #[test]
    fn refresh_updates_restore_snapshot_when_window_is_temporarily_overridden() {
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Fullscreen;
        let mut restore = WindowRestoreSnapshot {
            snapshot: Some(WindowRestoreState {
                geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
                fullscreen_output: None,
                previous: None,
            }),
        };
        let mut policy_state = WindowPolicyState {
            applied: WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal),
            locked: false,
        };

        refresh_window_policy(
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal),
            &mut layout,
            &mut mode,
            &mut restore,
            &mut policy_state,
        );

        assert_eq!(layout, WindowLayout::Floating);
        assert_eq!(mode, WindowMode::Fullscreen);
        let Some(snapshot) = restore.snapshot else {
            panic!("restore snapshot");
        };
        assert_eq!(snapshot.layout, WindowLayout::Tiled);
    }

    #[test]
    fn refresh_updates_nested_restore_snapshot_policy_frames() {
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Fullscreen;
        let mut restore = WindowRestoreSnapshot {
            snapshot: Some(WindowRestoreState {
                geometry: WindowSceneGeometry { x: 40, y: 50, width: 900, height: 700 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Maximized,
                fullscreen_output: None,
                previous: Some(Box::new(WindowRestoreState {
                    geometry: WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 },
                    layout: WindowLayout::Floating,
                    mode: WindowMode::Normal,
                    fullscreen_output: None,
                    previous: None,
                })),
            }),
        };
        let mut policy_state = WindowPolicyState {
            applied: WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal),
            locked: false,
        };

        refresh_window_policy(
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal),
            &mut layout,
            &mut mode,
            &mut restore,
            &mut policy_state,
        );

        let Some(snapshot) = restore.snapshot else {
            panic!("restore snapshot");
        };
        assert_eq!(snapshot.layout, WindowLayout::Floating);
        let Some(previous) = snapshot.previous else {
            panic!("previous restore snapshot");
        };
        assert_eq!(previous.layout, WindowLayout::Tiled);
        assert_eq!(previous.mode, WindowMode::Normal);
    }

    #[test]
    fn repeated_temporary_mode_entry_keeps_original_restore_snapshot() {
        let scene_geometry = WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 };
        let mut fullscreen_target = WindowFullscreenTarget::default();
        let mut restore = WindowRestoreSnapshot::default();
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Normal;
        let policy_state = WindowPolicyState::default();

        enter_temporary_window_mode(
            &scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            layout,
            &mut mode,
            WindowMode::Maximized,
            None,
        );

        let mut altered_scene_geometry = scene_geometry.clone();
        altered_scene_geometry.x = 500;
        altered_scene_geometry.y = 600;
        enter_temporary_window_mode(
            &altered_scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            layout,
            &mut mode,
            WindowMode::Maximized,
            None,
        );

        restore_window_state(
            &policy_state,
            &mut altered_scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            &mut layout,
            &mut mode,
        );

        assert_eq!(altered_scene_geometry, scene_geometry);
        assert_eq!(layout, WindowLayout::Floating);
        assert_eq!(mode, WindowMode::Normal);
        assert!(restore.snapshot.is_none());
    }

    #[test]
    fn nested_temporary_modes_restore_in_lifo_order() {
        let initial_geometry = WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 };
        let mut scene_geometry = initial_geometry.clone();
        let mut fullscreen_target = WindowFullscreenTarget::default();
        let mut restore = WindowRestoreSnapshot::default();
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Normal;
        let policy_state = WindowPolicyState::default();

        enter_temporary_window_mode(
            &scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            layout,
            &mut mode,
            WindowMode::Maximized,
            None,
        );
        scene_geometry = WindowSceneGeometry { x: 0, y: 0, width: 1920, height: 1080 };

        enter_temporary_window_mode(
            &scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            layout,
            &mut mode,
            WindowMode::Fullscreen,
            Some(OutputName::from("HDMI-A-1")),
        );
        scene_geometry = WindowSceneGeometry { x: 0, y: 0, width: 2560, height: 1440 };

        restore_window_state(
            &policy_state,
            &mut scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            &mut layout,
            &mut mode,
        );

        assert_eq!(mode, WindowMode::Maximized);
        assert_eq!(scene_geometry, WindowSceneGeometry { x: 0, y: 0, width: 1920, height: 1080 });
        assert!(fullscreen_target.output.is_none());
        assert!(restore.snapshot.is_some());

        restore_window_state(
            &policy_state,
            &mut scene_geometry,
            &mut fullscreen_target,
            &mut restore,
            &mut layout,
            &mut mode,
        );

        assert_eq!(mode, WindowMode::Normal);
        assert_eq!(scene_geometry, initial_geometry);
        assert!(restore.snapshot.is_none());
    }

    #[test]
    fn locked_policy_ignores_later_refreshes() {
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Normal;
        let mut restore = WindowRestoreSnapshot::default();
        let mut policy_state = WindowPolicyState::default();

        apply_window_policy(
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Normal),
            &mut layout,
            &mut mode,
            &mut policy_state,
        );
        layout = WindowLayout::Floating;
        lock_window_policy(layout, mode, &mut policy_state);
        refresh_window_policy(
            WindowPolicy::new(WindowLayout::Tiled, WindowMode::Fullscreen),
            &mut layout,
            &mut mode,
            &mut restore,
            &mut policy_state,
        );

        assert_eq!(layout, WindowLayout::Floating);
        assert_eq!(mode, WindowMode::Normal);
        assert_eq!(
            policy_state.applied,
            WindowPolicy::new(WindowLayout::Floating, WindowMode::Normal)
        );
    }

    #[test]
    fn sync_window_background_role_inserts_and_clears_background_component() {
        let mut world = World::default();
        let entity = world.spawn_empty().id();
        let mut role = WindowRole::Managed;
        let mut scene_geometry = WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 };
        let mut fullscreen_target = WindowFullscreenTarget::default();
        let mut layout = WindowLayout::Floating;
        let mut mode = WindowMode::Normal;

        {
            let mut commands = world.commands();
            sync_window_background_role(
                &mut commands,
                entity,
                Some(OutputId(7)),
                WindowBackgroundState::new(
                    &mut role,
                    &mut scene_geometry,
                    &mut fullscreen_target,
                    &mut layout,
                    &mut mode,
                ),
                None,
            );
        }
        world.flush();

        let Some(background) = world.get::<OutputBackgroundWindow>(entity) else {
            panic!("background role should exist");
        };
        let background = background.clone();
        assert_eq!(background.output, OutputId(7));
        assert_eq!(role, WindowRole::OutputBackground);
        assert_eq!(mode, WindowMode::Fullscreen);

        {
            let mut commands = world.commands();
            sync_window_background_role(
                &mut commands,
                entity,
                None,
                WindowBackgroundState::new(
                    &mut role,
                    &mut scene_geometry,
                    &mut fullscreen_target,
                    &mut layout,
                    &mut mode,
                ),
                Some(background),
            );
        }
        world.flush();

        assert!(world.get::<OutputBackgroundWindow>(entity).is_none());
        assert_eq!(role, WindowRole::Managed);
        assert_eq!(scene_geometry, WindowSceneGeometry { x: 10, y: 20, width: 800, height: 600 });
        assert_eq!(layout, WindowLayout::Floating);
        assert_eq!(mode, WindowMode::Normal);
    }
}
