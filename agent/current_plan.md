# Nekoland Current Plan

Date: 2026-03-14

This file tracks only the work that is still unfinished or newly active.

Archived completed work lives in [`agent/completed_plan.md`](/home/misaka/Code/nekoland/agent/completed_plan.md), including:

- the completed ECS modernization phases
- the completed event/request semantics and queue-unification work
- the completed fallible-system work
- the completed Phase 7 layer/output finalization
- the completed backend redesign phases B1-B7
- the completed high-level control-plane migration and cleanup
- the completed typed-selector and stable-control-id hardening pass
- the completed window layout / mode / stacking separation migration

## Current Status

New active work has started around the combined workspace/output redesign program:

1. infinite workspace viewport foundation
2. per-output active workspace
3. output background role

Most recently completed:

- Phase 1: `WindowLayout` and `WindowMode` are now separate runtime concepts across shell,
  render, protocol, IPC, and the covered tests.
- Phase 2: floating placement bookkeeping now lives in `WindowPlacement`; the old global
  `PositionedFloatingWindows` sidecar has been removed.
- Phase 3: maximize/fullscreen restore data now lives on the window entity through
  `WindowRestoreSnapshot`; the old global restore map has been removed.
- Phase 4: stacking is now tracked per workspace instead of through one global z-order list.
- Phase 5: typed window rules now resolve default layout/mode policy at the WM boundary for XDG
  and X11 windows, including metadata-driven refreshes while the window is still following its
  default policy.
- Phase 6: tiling now uses a real workspace-scoped tile tree and applies tiled base geometry
  through the shell layout pass instead of relying on placeholder comments/no-op systems.

Next up:

- Active plan: execute the ordered rollout described in
  [`agent/workspace_output_rollout_plan.md`](/home/misaka/Code/nekoland/agent/workspace_output_rollout_plan.md).
- Future optional follow-up: external shell mode, where `nekoland` acts as compositor/session
  host for an external shell such as QuickShell instead of embedding shell UI responsibilities.
  Design entry:
  [`agent/external_shell_mode_design.md`](/home/misaka/Code/nekoland/agent/external_shell_mode_design.md)
  This follow-up must stay aligned with the viewport/output model, especially the distinction
  between output-local shell UI surfaces and viewport-projected workspace windows.
- Future optional follow-up: add richer tiling-tree controls such as sibling swap, subtree rotate,
  and explicit attach/detach targeting on top of the existing split-axis controls once the
  viewport/scene split stops moving the geometry foundations underneath them.

## Active Plan: Workspace / Output Redesign Program

### Design Goals

- sequence the three related designs by dependency instead of implementing them as one large
  cross-cutting rewrite.
- establish scene-space window geometry before replacing global workspace routing.
- finish output/workspace routing before adding output background role.
- keep the compositor runnable and testable after each phase.

### Target Runtime Model

- Phase 1
  - scene-space window coordinates plus output viewport projection
- Phase 2
  - output-scoped current workspace and focused-output routing
- Phase 3
  - output-scoped background window role on top of the stabilized output-aware render path

### Architectural Rules

- scene-space geometry and presentation geometry must not be conflated in one field.
- output-local UI surfaces must stay separate from workspace-scene windows.
- no phase should bypass the high-level control plane by introducing fresh low-level request
  queues for user-facing actions.
- each phase must leave behind clear query/IPC semantics instead of temporary ambiguous fields.
- each phase must land the unit/integration/regression tests needed to hold its new semantics in
  place before the next phase starts.

### Migration Phases

1. Phase 1: infinite workspace viewport foundation
   - detailed design:
     [`agent/infinite_workspace_viewport_design.md`](/home/misaka/Code/nekoland/agent/infinite_workspace_viewport_design.md)
2. Phase 2: per-output active workspace
   - detailed design:
     [`agent/per_output_active_workspace_design.md`](/home/misaka/Code/nekoland/agent/per_output_active_workspace_design.md)
3. Phase 3: output background window
   - detailed design:
     [`agent/output_background_design.md`](/home/misaka/Code/nekoland/agent/output_background_design.md)
4. Ordered implementation details and validation gates:
   [`agent/workspace_output_rollout_plan.md`](/home/misaka/Code/nekoland/agent/workspace_output_rollout_plan.md)

### Acceptance Criteria

- the three phases can be implemented and validated independently.
- Phase 1 completion no longer leaves geometry semantics ambiguous.
- Phase 2 completion removes the global single-active-workspace assumption from output-visible
  behavior.
- Phase 3 completion adds background role without reintroducing geometry or routing ambiguity.

## Notes

- The window layout / mode / stacking separation migration is complete; this new plan builds on the
  resulting `WindowPolicyState`, `WindowPlacement`, `WindowRestoreSnapshot`, and
  `WorkspaceTilingState` foundations.
- The ordered execution entry point now lives in
  [`agent/workspace_output_rollout_plan.md`](/home/misaka/Code/nekoland/agent/workspace_output_rollout_plan.md).
- Completed historical work remains archived in
  [`agent/completed_plan.md`](/home/misaka/Code/nekoland/agent/completed_plan.md).
