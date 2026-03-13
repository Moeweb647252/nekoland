# Control Plane

The compositor's user-facing control plane is layered so callers operate on high-level window,
workspace, and output objects instead of assembling low-level transport-shaped requests.

## High-Level Entry Points

New user-facing control actions should start from the ECS-facing high-level control resources and
facades:

- `PendingWindowControls` plus `WindowOps` / `WindowControlApi`
- `PendingWorkspaceControls` plus `WorkspaceOps` / `WorkspaceControlApi`
- `PendingOutputControls` plus `OutputOps` / `OutputControlApi`

These are the intended entry points for:

- IPC command handling
- keybindings
- tests that want to model user-facing control actions
- future scripting or automation surfaces

## Typed Selectors and Boundary Parsing

The control plane now distinguishes user-facing boundary identifiers from internal runtime
selection.

Boundary-facing identifiers are wrapped in explicit types:

- `SurfaceId`
- `WorkspaceName`
- `OutputName`

Runtime-facing selectors are also typed:

- `WindowSelector`
- `WorkspaceLookup` / `WorkspaceSelector`
- `OutputSelector`

The rule is:

- IPC/config/keybinding layers may still accept strings and plain numbers
- those boundary values should be parsed into typed IDs or typed selectors immediately
- internal control APIs should prefer typed selectors, typed IDs, entities, and query markers over
  raw `String` and `u64`

## State vs. One-Shot Actions

Window control is split into two layers:

- persistent desired state
  - `WindowPlacement`
  - used for geometry-like intent such as floating position and size
- one-shot actions
  - staged through `PendingWindowControls`
  - used for edge-triggered actions such as focus and close

This keeps control-plane callers close to the common Bevy pattern of editing higher-level state and
letting scheduled systems reconcile it into runtime implementation details.

## Runtime Reconciliation

High-level controls are not the final implementation state.

Scheduled systems reconcile them into:

- shell runtime state
- `SurfaceGeometry`
- focus updates
- protocol close behavior
- backend-facing output application

That boundary is intentional: callers describe what they want to happen, while shell/protocol/
backend systems decide how to realize it.

For existing runtime objects, the control plane also leans on ECS query semantics where that makes
sense. In particular, the active workspace is now projected through the `ActiveWorkspace` marker
component so runtime-local code can query it directly instead of relying only on string lookup.

## Allowed Low-Level Bridges

Two low-level bridge queues remain on purpose:

- `PendingWindowServerRequests`
  - internal protocol bridge for finalized close requests
- `PendingOutputServerRequests`
  - internal backend bridge for finalized output application

These queues are not meant to be the public control surface. New user-facing actions should not
write them directly.

`PendingWorkspaceServerRequests` has been removed because workspace control no longer needs a
separate low-level transport layer.

## Rule of Thumb

When adding a new control action:

1. Add it to the high-level control resource or façade first.
2. Prefer typed selectors or typed IDs over raw strings and primitive identifiers.
3. Reconcile it in shell/backend/protocol systems.
4. Only add a low-level bridge queue if the action must cross into a narrower protocol or backend
   implementation boundary after high-level reconciliation has already happened.
