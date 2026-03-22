# Wayland App Architecture TODO

Parent milestone: [Wayland SubApp Boundary](../TODO.md)

## Goal

Build a dedicated `wayland` subapp that owns protocol runtime, backend runtime, raw seat/input
 extraction, output lifecycle, surface/buffer tracking, presentation, and protocol-facing frame
 feedback. The `wayland` subapp should act as the platform boundary of the compositor while the
 `main app` remains the authoritative shell world.

## Architecture Constraints

- [x] `wayland` owns protocol objects, backend handles, and output/present runtime state
- [x] `wayland` does not own shell policy, workspace state, focus rules, or layout decisions
- [x] `wayland` does not consume cross-world `Entity` values from `main app` or `render`
- [x] Cross-app communication uses explicit mailbox resources only
- [x] `wayland` may import compiled frame outputs for presentation, but render-world internals stay
      out of the subapp
- [x] The subapp is split into explicit phases such as:
      `poll/extract -> normalize/apply -> present -> feedback -> cleanup`

## Cross-App Mailboxes

- [x] Read external events and normalized platform snapshots into `WaylandIngress`
- [x] Accept shell-directed protocol/backend actions through `WaylandCommands`
- [x] Accept render outputs through `CompiledOutputFrames`
- [x] Emit present-time results through `WaylandFeedback`
- [x] Keep the mailbox contract stable enough that `main app` and `render subapp` do not need
      direct access to protocol/backend internals
- [x] Feed main-world compatibility resources from `WaylandIngress` / `WaylandFeedback` mailbox
      mirrors instead of direct per-resource sync-back where mailbox data already exists
- [x] Route backend-owned output reconciliation queues through explicit platform mailboxes instead
      of direct `wayland subapp -> main` queue clones
- [x] Let main-world output reconciliation consume `WaylandIngress.output_materialization`
      directly instead of hydrating separate compatibility queue resources first
- [x] Export backend output discovery/materialization through an explicit normalized plan mailbox
      instead of syncing raw backend extract queues into main-world reconciliation

## Work Items

### 1. Wayland SubApp Skeleton

- [x] Define the `wayland` subapp label and plugin entrypoint
- [x] Add a root-runner hook so the root app can run the subapp twice per frame where needed:
      once for poll/extract and once for present/feedback
- [x] Use the first extract pass to fan out `WaylandCommands` into platform-owned pending queues
      before main-world protocol/backend schedules run
- [x] Keep `WaylandCommands` one-way by consuming them inside the subapp without syncing the same
      pending command queues back into `main app`
- [x] Keep `WaylandIngress` and `WaylandFeedback` mailbox production inside the subapp boundary
- [x] Keep protocol/backend runtime non-send handles seeded directly inside the subapp boundary
- [x] Keep protocol event-queue state and dmabuf support subapp-local instead of mirroring them
      back through main-world resources each frame
- [x] Keep backend/protocol frame-local input and output queues subapp-local by clearing them in
      subapp cleanup
- [x] Stop bootstrap-seeding backend/protocol raw input and output queues from `main app`; keep
      them owned entirely by the subapp runtime
- [x] Stop seeding screenshot/readback request queues from `main app` into the subapp; keep the
      normal request lifecycle in subapp runtime plus mailbox feedback
- [x] Normalize backend raw input into a distinct platform-input mailbox inside the subapp
      instead of aliasing the same queue type across the boundary
- [x] Keep protocol clock advance inside the subapp extract phase
- [x] Keep `CompositorClock` subapp-owned after bootstrap instead of re-seeding it from
      `main app` every frame
- [x] Stop initializing main-world backend/protocol raw-input compatibility resources and
      `BackendStatus` where normal runtime ownership already lives inside the subapp
- [x] Stop initializing main-world present-audit / virtual-output / presentation-timeline
      compatibility resources where `WaylandFeedback` already provides the normal path
- [x] Stop pre-initializing `WaylandIngress` / `WaylandFeedback` in `main app`; let the subapp
      own mailbox production and insertion
- [x] Stop pre-initializing backend-owned wayland mailboxes in `main app`; let subapp sync-back
      insert them when needed
- [x] Move backend runtime bootstrap into the subapp build path so backend install and calloop
      registration no longer depend on main-app ownership
- [x] Move protocol runtime bootstrap into the subapp build path so Smithay server creation and
      calloop registration no longer depend on main-app ownership
- [x] Remove redundant ProtocolPlugin main-world initialization for resources already owned by
      shell/render mailbox paths
- [x] Stop pre-initializing protocol-originated output-event compatibility queues in `main app`;
      let `WaylandIngress` insertion own that path
- [x] Stop pre-initializing protocol server/xwayland compatibility state in `main app`; let
      `WaylandIngress` / `WaylandFeedback` insertion own that path
- [x] Remove main-world bootstrap seeding of `ProtocolServerState` / `XWaylandServerState`;
      read protocol runtime state from the `wayland` subapp or mailboxes instead
- [x] Stop seeding protocol-owned server, output, surface, and request snapshot resources from
      `main app` into the subapp; keep those runtime snapshots subapp-owned and mirror them back
      only through mailbox sync-back
- [x] Stop rehydrating main-world `ProtocolServerState` / `XWaylandServerState` every protocol
      tick from `WaylandIngress`; keep the mailbox as the canonical runtime path and leave only
      bootstrap compat where tests still need it
- [x] Keep protocol keyboard repeat/layout sync and dmabuf support application inside the subapp
- [x] Register protocol/backend calloop source installers through the subapp-owned registry
- [x] Move protocol/backend bootstrap into the subapp boundary
- [x] Keep the subapp responsible for its own runtime resources and non-send state

### 2. Poll And Extract

- [x] Poll calloop/backend/protocol runtime from inside the `wayland` subapp
- [x] Advance compositor frame timing from inside the `wayland` subapp extract phase
- [x] Extract raw seat/input events, output changes, surface lifecycle events, and backend updates
- [x] Convert external runtime transitions into frame-local pending queues
- [x] Avoid letting shell-facing code depend on Smithay or backend-specific extraction details

### 3. Surface And Buffer Tracking

- [x] Centralize `wl_surface` / xdg / layer / popup / dmabuf / shm tracking in the subapp
- [x] Maintain stable surface ids and lifecycle records that can cross app boundaries safely
- [x] Track buffer commits, frame callbacks, and readiness for presentation
- [x] Produce immutable surface snapshots suitable for `main app` and `render subapp`
      consumption
- [x] Carry attached/buffer-scale surface readiness through immutable platform surface snapshots
      so render-side bridge preparation no longer depends on direct main-world buffer queries
- [x] Carry surface content versions through immutable platform surface snapshots so render-side
      extract/bridge paths no longer depend on direct main-world content-version queries

### 4. Output And Backend Runtime

- [x] Keep backend descriptors, output discovery, mode/configuration, and output ownership in the
      subapp
- [x] Normalize output geometry into stable snapshot resources for input-side focus/clamp logic
      without leaking backend runtime handles
- [x] Stop maintaining a main-world backend-owned `OutputSnapshotState` compatibility resource in
      the normal runtime path once shell/input/render consume normalized output snapshots through
      `WaylandIngress`
- [x] Refresh output-geometry snapshots and primary-output selection from backend-owned output
      snapshots inside the subapp before handing them to `WaylandIngress`
- [x] Stop maintaining main-world backend-side primary-output synchronization in the normal
      runtime path once shell/backend consumers prefer `WaylandIngress.primary_output`
- [x] Export primary-output selection through `WaylandIngress` so shell layout policy does not need
      to read backend-owned platform resources directly
- [x] Prefer `WaylandIngress.primary_output` in backend present-surface extraction instead of
      deriving present-time target fallback from main-world `PrimaryOutputState` first
- [x] Prefer `WaylandIngress.primary_output` when resolving backend-side output control selectors
      instead of relying on main-world `PrimaryOutputState` first
- [x] Prefer `WaylandIngress` / `ShellRenderInput` when deriving backend present-surface
      snapshots in main-world compatibility schedules, leaving direct
      `PrimaryOutputState` / `SurfacePresentationSnapshot` reads as fallback-only
- [x] Stop falling back to main-world `PrimaryOutputState` / `SurfacePresentationSnapshot` in
      backend present input extraction; use `WaylandIngress` / `ShellRenderInput` or
      mailbox-default state instead
- [x] Keep backend-specific present logic behind a platform-facing interface
- [x] Generate config-driven output enable/disable/configure requests from backend-owned output
      snapshots inside the subapp before main-world ECS reconciliation
- [x] Translate shell-facing output enable/disable/configure requests into backend-normalized
      event/update queues inside the subapp before main-world ECS reconciliation
- [x] Build a normalized output materialization plan inside the subapp so main-world output ECS
      reconciliation only applies planned create/update/disconnect operations
- [x] Isolate winit, DRM, virtual-output, and future backend differences inside the subapp

### 5. Seat And Input Extraction

- [x] Keep raw keyboard, pointer, touch, gesture, and seat state in the `wayland` subapp
- [x] Normalize raw input into shell-facing events before handing them to `main app`
- [x] Let input decoding consume platform-input mailboxes directly instead of synchronizing a
      separate main-world raw platform-input queue first
- [x] Leave high-level policy, bindings, and semantic actions to `main app`
- [x] Ensure pointer focus/input routing can consume exported output geometry snapshots without
      depending on backend-owned output runtime views
- [x] Ensure shell focus/pointer output context can consume exported output geometry snapshots
      without querying backend-owned output runtime views
- [x] Let shell focus consume `WaylandIngress.output_snapshots` directly instead of maintaining
      extra shell-local output/surface snapshot compatibility copies
- [x] Prefer `WaylandIngress.primary_output` across shell viewport/layout/workspace/window
      routing paths instead of relying on the main-world `PrimaryOutputState` compatibility
      resource first

### 6. Main-App Boundary

- [x] Export `WaylandIngress` from the subapp boundary instead of building it in main-world
      protocol/backend schedules
- [x] Accept shell-driven window/popup/output-side platform requests through `WaylandCommands`
      fan-out owned by the subapp boundary
- [x] Accept protocol-side seat-input injections through `WaylandCommands` instead of direct
      main-world writes into raw subapp protocol-input queues
- [x] Move main-world synthetic/test seat-input pumps onto
      `WaylandCommands.pending_protocol_input_events` so raw protocol-input queues remain owned
      by the subapp even in integration scenarios
- [x] Let `wayland subapp` extract present-time pointer and surface-presentation state from
      `ShellRenderInput` instead of directly cloning separate main-world compatibility resources
- [x] Stop falling back to direct main-world pointer / surface-presentation resource clones during
      wayland/backend extract; use `ShellRenderInput` or mailbox-default state instead
- [x] Route shell startup/command launch environment checks through `WaylandIngress` instead of
      direct protocol server resources
- [x] Feed protocol-originated window-control requests through `WaylandIngress` instead of direct
      subapp-to-main resource sync-back
- [x] Let shell window-control handling consume `WaylandIngress.pending_window_controls`
      directly instead of synchronizing a separate main-world compatibility queue first
- [x] Feed protocol-originated output events through `WaylandIngress` instead of direct
      subapp-to-main resource sync-back
- [x] Let shell lifecycle/configure systems consume protocol-originated `xdg/x11/layer` request
      queues from `WaylandIngress` directly, keeping main-world queue resources only for
      deferred/test compatibility
- [x] Stop pre-initializing main-world `xdg/x11/layer` protocol-request compatibility queues in
      `ShellPlugin`; keep them optional and instantiate local queues only where deferred handling
      or tests still need them
- [x] Hydrate legacy protocol/cursor/surface compatibility resources from `WaylandIngress`
      instead of direct subapp-to-main per-resource sync-back where the mailbox already carries
      the same data
- [x] Accept shell-driven actions such as configure, focus handoff, seat state changes, popup
      grabs, output control requests, and protocol replies through `WaylandCommands`
- [x] Remove direct shell/protocol coupling where shell systems currently reach into protocol or
      backend resources
- [x] Keep shell policy authoritative while the subapp remains authoritative for protocol/runtime
      side effects

### 7. Present Boundary

- [x] Consume `CompiledOutputFrames` instead of render-internal planning resources
- [x] Present output-local compiled frames through backend-specific executors
- [x] Keep output damage, screenshot/readback delivery, and presentation timing inside the subapp
- [x] Ensure `wayland` is the only layer that turns a compiled frame into real output submission

### 8. Protocol Feedback And Replies

- [x] Emit frame callbacks from the subapp after successful present
- [x] Emit presentation feedback and screencopy completion from the subapp
- [x] Keep output-management, data-device, and clipboard protocol replies inside the platform
      boundary
- [x] Ensure feedback/results are mirrored to `WaylandFeedback` from the subapp boundary for shell
      or tooling consumption
- [x] Mirror presentation timelines, present audit snapshots, screenshot completion, and
      virtual-output capture through `WaylandFeedback` instead of separate direct main-world
      resource copies
- [x] Mirror pending screenshot/readback requests through `WaylandFeedback` so present-time queue
      mutation no longer needs a separate direct main-world sync-back
- [x] Stop hydrating main-world `PendingScreenshotRequests` compatibility state from
      `WaylandFeedback`; let shell/render consume feedback-owned screenshot requests directly
- [x] Mirror backend runtime descriptors through `WaylandFeedback` as ecs-neutral snapshots
      instead of direct `BackendStatus` sync
- [x] Mirror clipboard, primary-selection, and drag-and-drop state through `WaylandFeedback`
      instead of direct subapp-to-main per-resource sync-back
- [x] Stop hydrating main-world clipboard, primary-selection, and drag-and-drop compatibility
      resources from `WaylandFeedback`; keep selection state canonical in the feedback mailbox
- [x] Keep `WaylandFeedback` as the canonical source for presentation/audit/completed-screenshot/
      capture queries instead of hydrating extra main-world compatibility resources
- [x] Let tooling/query paths prefer `WaylandFeedback` for present-time and selection snapshots
      instead of depending on direct compatibility resources first
- [x] Make IPC selection snapshots feedback-only instead of falling back to main-world
      clipboard/primary-selection compatibility resources
- [x] Stop depending on main-world `BackendOutputRegistry` in startup/tooling query paths; use
      normalized output snapshots and platform-facing mailbox data instead

### 9. Render Interop Contract

- [x] Define the render-facing surface texture/buffer import contract without exposing protocol
      objects to render
- [x] Export platform import capabilities through `WaylandIngress` so render-side import
      preparation can gate dma-buf readiness on wayland-owned capability snapshots
- [x] Keep SHM and dma-buf import preparation behind a wayland-owned abstraction
- [x] Export enough per-surface dma-buf / external-texture import metadata from the wayland
      boundary for render/backends to perform actual non-SHM imports without consulting Smithay
      state directly
- [x] Export only stable resource descriptors that render can prepare or import
- [x] Avoid letting render directly depend on Smithay backend/present code

### 10. Migration Of Existing Code

- [x] Fold current protocol/backend/input runtime pieces into the `wayland` subapp boundary
- [x] Separate low-level input extraction from shell-level action mapping
- [x] Move frame callback, presentation feedback, screenshot completion, and protocol-side present
      bookkeeping under the subapp
- [x] Reduce direct dependencies from shell/render code onto protocol/backend resources
- [x] Replace any cross-boundary `Entity` assumptions with stable ids and snapshots

### 11. Verification

- [x] Add tests for `wayland` mailbox extraction and command application
- [x] Add tests for surface/output snapshot normalization
- [x] Add tests for present/feedback flow using compiled frame inputs
- [x] Add at least one smoke path that exercises:
      `wayland poll -> main shell update -> render update -> wayland present`
