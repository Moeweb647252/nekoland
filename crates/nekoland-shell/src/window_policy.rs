use bevy_ecs::prelude::{Commands, Entity};
use nekoland_ecs::components::WindowRestoreState;
use nekoland_ecs::components::{
    OutputBackgroundWindow, WindowFullscreenTarget, WindowLayout, WindowMode, WindowPolicy,
    WindowPolicyState, WindowRestoreSnapshot, WindowRole, WindowSceneGeometry,
};
use nekoland_ecs::selectors::OutputName;

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

    if let Some(snapshot) = restore.snapshot.as_mut()
        && snapshot.layout == previous.layout
        && snapshot.mode == previous.mode
    {
        snapshot.layout = policy.layout;
        snapshot.mode = policy.mode;
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

pub fn sync_window_background_role(
    commands: &mut Commands,
    entity: Entity,
    desired_output: Option<OutputName>,
    window: WindowBackgroundState<'_>,
    current_background: Option<OutputBackgroundWindow>,
) {
    let WindowBackgroundState { role, scene_geometry, fullscreen_target, layout, mode } = window;
    let desired_output = desired_output.map(|output| output.as_str().to_owned());
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
        OutputBackgroundWindow, WindowFullscreenTarget, WindowRestoreState, WindowRole,
        WindowSceneGeometry,
    };
    use nekoland_ecs::selectors::OutputName;

    use super::{
        WindowBackgroundState, WindowLayout, WindowMode, WindowPolicy, WindowPolicyState,
        WindowRestoreSnapshot, apply_window_policy, lock_window_policy, refresh_window_policy,
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
                Some(OutputName::from("Virtual-1")),
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
        assert_eq!(background.output, "Virtual-1");
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
