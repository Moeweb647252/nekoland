use nekoland_ecs::components::{
    WindowLayout, WindowMode, WindowPolicy, WindowPolicyState, WindowRestoreSnapshot,
};

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
        if snapshot.layout == previous.layout && snapshot.mode == previous.mode {
            snapshot.layout = policy.layout;
            snapshot.mode = policy.mode;
        }
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

#[cfg(test)]
mod tests {
    use nekoland_ecs::components::{SurfaceGeometry, WindowRestoreState};

    use super::{
        WindowLayout, WindowMode, WindowPolicy, WindowPolicyState, WindowRestoreSnapshot,
        apply_window_policy, lock_window_policy, refresh_window_policy,
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
                geometry: SurfaceGeometry { x: 10, y: 20, width: 800, height: 600 },
                layout: WindowLayout::Floating,
                mode: WindowMode::Normal,
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
        assert_eq!(restore.snapshot.expect("restore snapshot").layout, WindowLayout::Tiled);
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
}
