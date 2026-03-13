# Nekoland Completed Plan Archive

Date: 2026-03-13

This file archives the major plan items that have already been completed and no longer need to
stay in `current_plan.md`.

## Completed ECS Modernization

The internal ECS model was migrated away from manual joins and duplicated filtering toward
relationship-driven modeling, default query filtering, and stronger structural invariants.

Completed milestones:

- Entity index foundation
  - Added `EntityIndex` and `rebuild_entity_index_system`
  - Replaced the main `surface_id` scan hotspots with index-first lookup plus safe fallback
- Required components
  - Added low-risk `#[require(...)]` invariants for `XdgWindow`, `XdgPopup`,
    `LayerShellSurface`, and `X11Window`
- Popup parent relationships
  - Replaced popup parent bookkeeping with `ChildOf(parent_window)`
  - Removed the transitional `XdgPopup.parent_surface` field
  - Updated render/protocol/IPC consumers to derive popup parentage from relationships
- Workspace/window relationships
  - Replaced workspace membership with `ChildOf(workspace)`
  - Removed `LayoutSlot` entirely from ECS/runtime/tests
  - Unified workspace-membership helpers across shell/render/protocol/IPC
- Default filtering with `Disabled`
  - Non-active workspace trees now use `Disabled`
  - Updated global consumers and “see everything” paths to explicitly mention disabled entities
- Layer/output checkpoint
  - Introduced non-owning `LayerOnOutput -> OutputLayers`
  - Moved edge-facing output naming into `DesiredOutputName`
  - Switched arrangement/work-area consumers to be relationship-first

Validation that was completed for the ECS migration:

- `cargo check --workspace`
- `cargo test --workspace`
- Targeted crate and integration suites during each migration phase

## Completed Phase 7: Layer -> Output Finalization

The layer/output model now has explicit runtime semantics instead of relying on an implicit
"largest output" fallback.

Completed milestones:

- Kept the layer/output relationship non-owning
  - `LayerOnOutput -> OutputLayers` remains the runtime truth
  - output disappearance detaches the layer instead of destroying it
- Pushed edge-facing output naming into `DesiredOutputName`
  - `LayerShellSurface` no longer carries the output name directly
- Added explicit primary-output state
  - introduced `PrimaryOutputState`
  - backend now derives a stable primary output from configured enabled outputs with a largest-size
    fallback when config does not resolve
- Finalized detached-layer semantics
  - layers that name a missing output stay detached
  - detached named layers do not silently fall back to the primary output
  - only layers with no desired output name use primary-output fallback
- Updated arrangement/work-area consumers to use the finalized rule set
- Added regression coverage for:
  - detached named layers staying detached
  - detached named layers not shrinking the work area
  - explicit primary-output selection overriding size fallback
  - late output appearance/disappearance updating layer relationships correctly

Validation completed:

- `cargo test -p nekoland-shell -p nekoland-backend`
- `cargo test -p nekoland --test inprocess_layer_shell`
- `cargo test --workspace`

## Completed Fallible-System Work

Completed milestones:

- Installed a default Bevy ECS warn handler in `NekolandApp::new`
- Converted `hot_reload_system` to a real fallible system
- Preserved live config state on invalid reload while still surfacing errors through the handler
- Converted backend configured-output synchronization into a meaningful fallible boundary
  - invalid configured output modes are skipped instead of emitting broken runtime requests
  - invalid modes are reported through the Bevy ECS error handler as configuration errors
  - the last applied output snapshot still advances so valid configuration changes continue to apply
- Added backend primary-output state synchronization
  - backend publishes explicit `PrimaryOutputState`
  - primary output prefers the first enabled configured output and falls back deterministically when
    config does not match runtime outputs
- Audited observer adoption and intentionally deferred it
  - observer/hook work remains out of scope for now
  - `EntityIndex` stays schedule-maintained because that is still the clearer model in this codebase

Validation completed:

- `cargo test -p nekoland-config -p nekoland-core -p nekoland-backend`
- `cargo check --workspace`
- `cargo test --workspace`

## Completed Event / Request Semantics Work

The event/request model now has explicit semantic categories without renaming the concrete
payload types.

Completed milestones:

- Added semantic marker traits:
  - `ProtocolEvent`
  - `BackendEvent`
  - `CompositorRequest`
  - `CompositorEvent`
  - `SubscriptionEvent`
- Implemented the marker traits for existing concrete payload types across:
  - `nekoland-ecs`
  - `nekoland-protocol`
  - `nekoland-ipc`
- Added compile-time classification tests for the semantic categories
- Replaced ad-hoc queue wrappers with a single `FrameQueue<T, Tag>` abstraction
- Converted the main request/event queues to semantic aliases over `FrameQueue`
- Folded the last input/audit queues onto the same `FrameQueue` model using distinct marker tags
- Made queue internals private and switched runtime/test code to the shared queue API:
  - `from_items`
  - `push`
  - `extend`
  - `drain`
  - `take`
  - `replace`
  - `clear`
  - `is_empty`
  - `len`
  - `iter`
  - `as_slice`

Validation completed:

- `cargo check --workspace`
- `cargo test -p nekoland-ecs -p nekoland-protocol -p nekoland-ipc -p nekoland-shell -p nekoland-backend -p nekoland-input`
- `cargo test --workspace`

## Completed Backend Redesign (B1-B7)

The backend subsystem was restructured from global `SelectedBackend`-style schedule gating into
an ECS-native manager/runtime model with explicit backend ownership.

Completed milestones:

- B1: split common backend logic out of the plugin wiring file
  - introduced [`common/outputs.rs`](/home/misaka/Code/nekoland/crates/nekoland-backend/src/common/outputs.rs)
  - introduced [`common/presentation.rs`](/home/misaka/Code/nekoland/crates/nekoland-backend/src/common/presentation.rs)
  - reduced [`plugin.rs`](/home/misaka/Code/nekoland/crates/nekoland-backend/src/plugin.rs) to wiring plus manager dispatch
- B2: introduced explicit backend ownership metadata
  - added `BackendId`
  - added `OutputBackend` as ECS ownership metadata for output entities
  - output materialization and request application now route through backend ownership instead of a
    global selected backend kind
- B3: introduced `BackendManager`
  - active backend instances are now owned by a manager resource
  - backend status is projected into ECS through `BackendStatus`
  - requested backends are parsed as a set, enabling multi-instance startup such as `drm,virtual`
- B4: folded Winit into a complete backend runtime
  - `WinitRuntime` now owns host-window input, window-state projection, nested rendering, and
    presentation timing
- B5: folded DRM into a complete backend runtime
  - `DrmRuntime` now owns tty session state, libinput drain, device discovery, GBM allocation,
    and DRM present/render state
- B6: folded Virtual into a coexisting backend runtime
  - `VirtualRuntime` now owns synthetic output publication, presentation timing, and frame capture
- B7: replaced broad schedule gating with manager dispatch
  - `ExtractSchedule` now dispatches through `backend_extract_system` and `backend_apply_system`
  - `PresentSchedule` now dispatches through `backend_present_system`
  - removed backend orchestration’s dependency on the old global `SelectedBackend` resource

Architectural outcomes:

- backend instances are full input+output runtime units
- output ownership is explicit in ECS
- common output/presentation normalization is shared across all backends
- backend work stays ECS-native through constrained extract/apply/present contexts
- backend-local state is primarily owned by runtime structs instead of scattered world-level
  orchestration

Validation completed:

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo test -p nekoland-backend`
- `cargo test -p nekoland --test startup_protocol --test virtual_output --test inprocess_keybindings --test inprocess_presentation_feedback`
- `cargo test --workspace`

## Completed High-Level Control Plane Migration

The main user-facing control paths were migrated away from transport-shaped low-level requests
toward higher-level ECS-native control resources plus facades.

Completed milestones:

- Added persistent high-level window control state
  - introduced `WindowPlacement`
  - made window geometry intent explicit instead of requiring callers to write low-level move/resize
    requests
- Added staged control resources:
  - `PendingWindowControls`
  - `PendingWorkspaceControls`
  - `PendingOutputControls`
- Added ergonomic control facades:
  - `WindowControlApi` / `WindowOps`
  - `WorkspaceControlApi` / `WorkspaceOps`
  - `OutputControlApi` / `OutputOps`
- Reconciled high-level window controls in shell runtime
  - move/resize/focus now resolve through the control plane
  - close is translated into the remaining protocol-facing bridge
- Updated floating layout and interactive move/resize
  - floating layout now reads `WindowPlacement`
  - pointer-driven interactive geometry updates write back into `WindowPlacement`
- Migrated the primary entry points to the high-level control plane:
  - keybindings
  - IPC command handling
  - workspace control flow
  - output control flow
- Narrowed old low-level window requests
  - `WindowServerAction` is now close-only
  - old shell-side focus/move/resize consumers were removed

Architectural outcomes:

- callers now operate on window/workspace/output control objects instead of assembling low-level
  request transport structs
- geometry-like operations are staged as high-level state and reconciled by systems
- low-level bridge queues are no longer the primary public control entry point

Validation completed:

- `cargo test -p nekoland-ecs -p nekoland-shell -p nekoland-input -p nekoland-ipc`
- `cargo test -p nekoland --test ipc_control_plane`
- `cargo test --workspace`

## Completed Window Layout / Mode / Stacking Separation

The window runtime model now treats layout policy, presentation mode, stacking, floating
placement, default policy resolution, and tiling state as separate concerns instead of overloading
one `WindowState` concept.

Completed milestones:

- Split mixed window state into separate layout and mode concepts
  - introduced `WindowLayout`
  - introduced `WindowMode`
  - kept user-facing state export through derived `WindowDisplayState`
- Moved floating placement intent onto the window entity
  - `WindowPlacement` now carries floating position/size intent directly
  - removed the old global floating placement sidecar
- Moved maximize/fullscreen restore snapshots onto the window entity
  - introduced entity-local `WindowRestoreSnapshot`
  - removed the old global restore map
- Finalized workspace-scoped stacking
  - `WindowStackingState` now tracks z-order per workspace
  - focus, render, and hit-testing now consume the same workspace-local ordering source
- Added typed window policy resolution at the WM boundary
  - introduced `WindowPolicy` and `WindowPolicyState`
  - compositor config now supports typed `window_rules`
  - XDG and X11 lifecycle paths now resolve/apply policy at create-time and metadata updates
- Implemented real workspace-scoped tiling state
  - introduced `WorkspaceTilingState` with a per-workspace binary tile tree
  - registered a real `tiling_layout_system`
  - tiled windows now receive base geometry from tree layout instead of placeholder comments
- Added explicit split-axis manipulation on top of the tiling tree
  - `WindowControl` can now stage `window split horizontal|vertical`
  - keybindings, IPC server, and `nekoland-msg` all expose the same split-axis action
  - tile trees now preserve explicit split edits across normal surface add/remove reconciliation

Architectural outcomes:

- layout produces base geometry only
- maximize/fullscreen remain post-layout constraints
- stacking is fully independent from layout
- window defaults are resolved once into typed policy and then tracked per entity
- tiled geometry is now driven by workspace-local tree state instead of window-state enums
- split manipulation reuses the existing high-level window control plane instead of introducing a
  second tiling-only request channel

Validation completed:

- `cargo fmt --all`
- `cargo check --workspace --tests`
- `cargo test -p nekoland-ecs -p nekoland-shell -p nekoland-input -p nekoland-ipc`
- `cargo test -p nekoland --test config_runtime --test inprocess_window_states --test ipc_control_plane --test ipc_config_subscription`
- `cargo test -p nekoland-msg`

## Completed Control Plane Cleanup

The remaining control-plane cleanup work was completed so the higher-level API is now the clear
public entry point and the leftover low-level queues are explicitly bridge-only.

Completed milestones:

- removed `PendingWorkspaceServerRequests` entirely
  - workspace control now flows only through `PendingWorkspaceControls`
- kept `PendingWindowServerRequests` and `PendingOutputServerRequests` only as documented internal
  bridge contracts
  - window close bridge remains protocol-facing
  - output request bridge remains backend-facing
- normalized selector ergonomics
  - improved the first-stage control-plane API before the later typed-selector hardening pass
- documented the intended control-plane boundary
  - added [`docs/control_plane.md`](/home/misaka/Code/nekoland/docs/control_plane.md)
  - linked the new note from [`docs/architecture.md`](/home/misaka/Code/nekoland/docs/architecture.md)

Architectural outcomes:

- high-level callers now have a clearer “select object, then stage action” API shape
- workspace control no longer carries a redundant low-level compatibility queue
- bridge-only low-level queues are explicitly scoped to protocol/backend boundaries
- the intended layering is now written down for future contributors

Validation completed:

- `cargo check --workspace`
- `cargo fmt --all`
- `cargo test --workspace`

## Completed Typed Selector and Stable Control ID Hardening

The control plane now parses loose boundary identifiers at the edge and uses typed selectors or
typed IDs through the internal control APIs.

Completed milestones:

- introduced typed boundary identifiers:
  - `SurfaceId`
  - `WorkspaceName`
  - `OutputName`
- introduced typed control selectors:
  - `WindowSelector`
  - `WorkspaceLookup`
  - `WorkspaceSelector`
  - `OutputSelector`
- updated high-level control resources to use typed IDs/selectors instead of raw `String`/`u64`
  defaults
- pushed boundary parsing to the edge:
  - IPC now parses string workspace/output targets into typed lookups/selectors
  - keybinding dispatch now does the same for workspace/output actions
- improved runtime-local control semantics:
  - added `ActiveWorkspace` as an explicit ECS marker component
  - `WorkspaceOps` can query the active workspace through ECS-facing semantics instead of relying
    only on string-oriented helpers
- extended output control selection with an explicit `Primary` selector
  - high-level primary-output controls defer until a concrete primary output is known
- removed the remaining ambiguous workspace API shape
  - control flow no longer depends on `named(...).switch_to()` style behavior to encode
    create-vs-switch semantics

Architectural outcomes:

- strings and raw integers remain valid boundary input, but they are no longer the default runtime
  selector vocabulary
- the control plane better distinguishes:
  - boundary names/ids
  - runtime selectors
  - existing-object query semantics
- missing-target behavior is encoded more explicitly in method names and selector types

Validation completed:

- `cargo check --workspace`
- `cargo test --workspace`

## Notes

- This archive intentionally summarizes completed outcomes instead of preserving the full
  step-by-step migration log that previously lived in `current_plan.md`.
- The remaining incomplete work and all newly active backend-design planning live in
  `agent/current_plan.md`.
