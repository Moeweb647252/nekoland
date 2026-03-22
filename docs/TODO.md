# TODO

## Architecture Target

Target runtime shape:

- root runner
- `main app` as the authoritative shell world
- `wayland` subapp for protocol/backend/input/present runtime
- `render` subapp for render-world extraction, compilation, execution, and readback

## Planning Docs

- [Shell App Architecture TODO](./milestones/shell-app-architecture.md)
- [Wayland App Architecture TODO](./milestones/wayland-app-architecture.md)
- [Render SubApp Architecture TODO](./milestones/render-subapp-architecture.md)

## Execution Order

Milestones are strictly serial.

Implementation order:

1. establish `main app` authority, root-runner order, and mailbox ownership
2. move platform/runtime responsibilities behind the `wayland` subapp boundary
3. move render-world responsibilities behind the `render` subapp boundary
4. upgrade render internals toward the stronger `bevy_render`-style architecture

## Milestones

### Milestone 1: Main App / Shell Authority

Status: completed

Goal: define `main app` as the authoritative shell world, fix the root-runner orchestration, and
lock the cross-app mailbox ownership model before pushing runtime responsibilities into subapps.

Depends on: none

Exit criteria:

- the root runner order is explicitly fixed
- shell-owned state and policy boundaries are documented
- mailbox names and ownership are fixed across `main app`, `wayland`, and `render`
- the shell architecture doc exists and is linked from this roadmap

Planning docs:

- [Shell App Architecture TODO](./milestones/shell-app-architecture.md)

- [x] Keep shell state and policy authoritative in `main app`
- [x] Define the root runner order:
      `wayland poll/extract -> main shell update -> render update -> wayland present/feedback`
- [x] Replace cross-app `Entity` usage with stable ids plus frame snapshots/mailboxes
- [x] Define mailbox resources for the app boundaries:
      `WaylandIngress`, `ShellRenderInput`, `CompiledOutputFrames`, `WaylandCommands`,
      `WaylandFeedback`
- [x] Document the shell/main-app implementation plan

### Milestone 2: Wayland SubApp Boundary

Status: in_progress

Goal: move protocol/backend/input/present runtime behind the `wayland` subapp boundary and ensure
`main app` only consumes normalized platform snapshots and emits explicit platform commands.

Depends on: Milestone 1

Exit criteria:

- the `wayland` subapp owns protocol objects, backend state, and present-time runtime state
- shell-facing platform inputs are normalized before they enter `main app`
- present, frame callback, presentation feedback, and screencopy reply handling are isolated in the
  `wayland` subapp

Planning docs:

- [Wayland App Architecture TODO](./milestones/wayland-app-architecture.md)

- [x] Define the `wayland` subapp label, plugin entrypoint, and extract/sync-back bridge
- [x] Add a root-runner hook so the `wayland` subapp can execute during extract and present phases
- [x] Replace present-facing cross-boundary `Entity` usage with stable ids plus normalized
      snapshots
- [x] Move `WaylandIngress` and `WaylandFeedback` mailbox production into the `wayland` subapp
- [x] Fan out `WaylandCommands` into platform-owned pending queues from the `wayland` subapp
- [x] Keep shell-owned `WaylandCommands` one-way by consuming them inside the `wayland` subapp
      without syncing the same pending command queues back into `main app`
- [x] Keep shell-owned output command and overlay state initialized in `ShellPlugin` instead of
      `BackendPlugin`, and let backend control application treat primary/focused output fallback
      resources as optional compatibility state
- [x] Let `wayland subapp` extract present-time pointer and surface-presentation state from
      `ShellRenderInput` instead of directly cloning separate main-world compatibility resources
- [x] Stop falling back to direct main-world pointer / surface-presentation resource clones during
      wayland/backend subapp extract; use `ShellRenderInput` or mailbox-default state instead
- [x] Route shell startup/command environment lookups through `WaylandIngress` instead of direct
      protocol server resources
- [x] Normalize output geometry into explicit snapshot resources for input-side routing and clamp
- [x] Feed normalized output snapshots into input via `WaylandIngress` instead of direct backend
      resources
- [x] Stop maintaining a main-world backend-owned `OutputSnapshotState` compatibility resource
      in the normal runtime path once shell/input/render consume normalized output snapshots
      through `WaylandIngress`
- [x] Consume platform input events directly from `WaylandIngress` inside the input schedule
      instead of synchronizing a separate main-world raw platform-input queue first
- [x] Refresh normalized output geometry and primary-output selection inside the `wayland`
      subapp from backend-owned output snapshots instead of relying only on main-world output
      queries and sync-back
- [x] Stop maintaining main-world backend-side primary-output synchronization in the normal
      runtime path once shell/backend consumers prefer `WaylandIngress.primary_output`
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
      `ShellPlugin`; keep them optional and only instantiate them where deferred handling or tests
      still need local queues
- [x] Route protocol-side seat-input injection through `WaylandCommands` instead of direct
      main-world writes into the `wayland` subapp's raw protocol-input queue
- [x] Move main-world synthetic/test seat-input pumps onto
      `WaylandCommands.pending_protocol_input_events` so raw protocol-input queues stay owned by
      the `wayland` subapp even in integration scenarios
- [x] Feed shell focus/pointer output context from normalized output snapshots instead of direct
      backend-owned output runtime views
- [x] Let shell focus consume `WaylandIngress.output_snapshots` directly instead of synchronizing
      separate shell-local output/surface snapshot compatibility resources first
- [x] Feed shell primary-output selection state from `WaylandIngress` instead of direct backend
      resources
- [x] Prefer `WaylandIngress.primary_output` across shell viewport/layout/workspace/window
      routing paths instead of relying on `PrimaryOutputState` compatibility state first
- [x] Prefer `WaylandIngress.primary_output` in backend present-surface extraction instead of
      deriving present-time target fallback from main-world `PrimaryOutputState` first
- [x] Prefer `WaylandIngress.primary_output` when resolving backend-side output control selectors
      instead of relying on main-world `PrimaryOutputState` first
- [x] Prefer `WaylandIngress` / `ShellRenderInput` when deriving backend present-surface snapshots
      in main-world compat schedules, keeping `PrimaryOutputState` /
      `SurfacePresentationSnapshot` only as fallback compatibility state
- [x] Stop falling back to main-world `PrimaryOutputState` / `SurfacePresentationSnapshot` in
      backend present input extraction; use `WaylandIngress` / `ShellRenderInput` or
      mailbox-default state instead
- [x] Export immutable platform surface snapshots through `WaylandIngress` and use them to seed
      render-facing import descriptors without exposing live protocol objects
- [x] Carry attached/buffer-scale surface readiness through platform surface snapshots so render
      no longer needs direct main-world buffer-state extraction for bridge preparation
- [x] Carry surface content versions through platform surface snapshots so render extract/bridge
      preparation stop querying main-world surface content components directly
- [x] Move compositor clock advance plus protocol keyboard/dmabuf runtime sync into the
      `wayland` subapp
- [x] Seed protocol/backend runtime non-send handles directly into the `wayland` subapp instead of
      only mirroring them from main-world schedules
- [x] Keep protocol event-queue state and dmabuf support subapp-local instead of re-synchronizing
      them back through main-world resources each frame
- [x] Register protocol/backend calloop source installers against the `wayland` subapp registry
      instead of the main-world runtime registry
- [x] Poll the shared calloop runtime from the `wayland` subapp extract phase instead of the
      outer root runner
- [x] Split the `wayland` subapp into explicit poll/extract, normalize/apply, present, feedback,
      and cleanup phase sets so platform work has a fixed internal order
- [x] Mirror presentation timelines, present audit, screenshot completion, and virtual-output
      capture through `WaylandFeedback` instead of keeping separate direct main-world copies
- [x] Mirror backend runtime descriptors through `WaylandFeedback` as ecs-neutral snapshots so
      integration paths stop depending on direct `BackendStatus` sync
- [x] Mirror clipboard, primary-selection, and drag-and-drop state through `WaylandFeedback` so
      protocol-side selection results no longer need direct per-resource sync-back
- [x] Stop hydrating main-world clipboard, primary-selection, and drag-and-drop compatibility
      resources from `WaylandFeedback`; keep selection state canonical in the feedback mailbox
- [x] Mirror pending screenshot/readback requests through `WaylandFeedback` so present-time queue
      mutation no longer needs a direct `wayland subapp -> main` clone
- [x] Stop hydrating main-world `PendingScreenshotRequests` compatibility state from
      `WaylandFeedback`; let shell/render consume feedback-owned screenshot requests directly
- [x] Stop hydrating main-world presentation/audit/completed-screenshot/virtual-capture
      compatibility resources from `WaylandFeedback`; keep mailbox data as the canonical
      present-time query path
- [x] Hydrate main-world protocol compatibility resources from `WaylandIngress` /
      `WaylandFeedback` mailboxes instead of direct per-resource sync-back for server, cursor,
      surface snapshot, presentation, screenshot, and capture state
- [x] Mirror backend output reconciliation through explicit platform mailboxes instead of direct
      `wayland subapp -> main` resource clones
- [x] Make main-world output reconciliation consume `WaylandIngress.output_materialization`
      directly instead of hydrating separate compatibility queue resources first
- [x] Replace raw backend output event/update queue sync-back with an explicit output
      materialization plan mailbox so main-world output reconciliation only applies a normalized
      platform plan
- [x] Keep backend/protocol raw frame-local input and output queues subapp-local by clearing them
      in `wayland` cleanup
- [x] Stop bootstrap-seeding backend/protocol raw input and output queues from `main app`; keep
      them owned entirely by the `wayland` subapp runtime
- [x] Stop seeding screenshot/readback request queues from `main app` into the `wayland`
      subapp; keep normal request lifecycle in subapp runtime plus mailbox feedback
- [x] Normalize backend raw input queues into a distinct platform-input mailbox inside the
      `wayland` subapp instead of aliasing the same queue type across the boundary
- [x] Keep `CompositorClock` subapp-owned after bootstrap instead of re-seeding it from
      `main app` every frame
- [x] Stop initializing main-world `BackendStatus` and raw protocol/backend input compat
      resources where normal runtime ownership already lives in the `wayland` subapp
- [x] Stop initializing main-world backend present compatibility resources where normal runtime
      ownership already lives in the `wayland` subapp
- [x] Stop initializing main-world present-audit / virtual-output / presentation-timeline
      compatibility resources where `WaylandFeedback` already provides the normal path
- [x] Stop pre-initializing `WaylandIngress` / `WaylandFeedback` in `main app`; let the
      `wayland` subapp own mailbox production and insertion
- [x] Stop pre-initializing backend-owned wayland mailboxes in `main app`; let subapp sync-back
      insert them when needed
- [x] Move backend runtime bootstrap into the `wayland` subapp build path so backend install and
      calloop registration no longer depend on main-app ownership
- [x] Move protocol runtime bootstrap into the `wayland` subapp build path so Smithay server
      creation and calloop registration no longer depend on main-app ownership
- [x] Remove redundant ProtocolPlugin main-world initialization for resources already owned by
      shell/render mailbox paths
- [x] Stop pre-initializing protocol-originated output-event compatibility queues in `main app`;
      let `WaylandIngress` insertion own that path
- [x] Stop pre-initializing protocol server/xwayland compatibility state in `main app`; let
      `WaylandIngress` / `WaylandFeedback` insertion own that path
- [x] Remove main-world bootstrap seeding of `ProtocolServerState` / `XWaylandServerState`;
      read protocol runtime state from `WaylandSubApp` or mailboxes instead
- [x] Stop seeding protocol-owned server, output, surface, and request snapshot resources from
      `main app` into the `wayland` subapp; keep those runtime snapshots subapp-owned and mirror
      them out only through mailbox sync-back
- [x] Stop rehydrating main-world `ProtocolServerState` / `XWaylandServerState` every protocol
      tick from `WaylandIngress`; keep the mailbox as the canonical runtime path and leave only
      bootstrap compat where tests still need it
- [x] Let IPC/tooling query paths prefer `WaylandFeedback` for present-time and selection
      snapshots instead of direct compatibility resources first
- [x] Make IPC selection snapshots feedback-only instead of falling back to main-world
      clipboard/primary-selection compatibility resources
- [x] Stop depending on main-world `BackendOutputRegistry` in startup/tooling query paths; use
      normalized output snapshots and platform-facing mailbox data instead
- [x] Translate output enable/disable/configure requests into backend-normalized event/update
      queues inside the `wayland` subapp instead of directly materializing ECS output entities
- [x] Generate config-driven output enable/disable/configure requests from backend output snapshots
      inside the `wayland` subapp instead of main-world output queries
- [x] Move Wayland protocol, backend runtime, seat/input extraction, and present flow into a
      `wayland` subapp
- [x] Stop leaking protocol/backend details into shell-facing data paths
- [x] Normalize surface, output, and input snapshots before they reach `main app`
- [x] Isolate present-time feedback, frame callbacks, and screencopy reply handling inside the
      `wayland` subapp

### Milestone 3: Render SubApp Boundary

Status: completed

Goal: move render extraction, graph compilation, execution, and readback into the `render`
subapp, and make `wayland` present compiled frame outputs instead of render-internal planning
resources.

Depends on: Milestone 2

Exit criteria:

- the `render` subapp owns render-world lifecycle and frame compilation responsibilities
- `ShellRenderInput` and `CompiledOutputFrames` are the only cross-boundary render mailboxes
- present no longer consumes render-internal planning resources directly

Planning docs:

- [Render SubApp Architecture TODO](./milestones/render-subapp-architecture.md)

- [x] Move render extraction, material/pipeline preparation, graph compilation, and frame
      execution into a `render` subapp
- [x] Define stable render-facing ids for outputs, surfaces, windows, layers, cursors, and scene
      items where needed
- [x] Move render-plan assembly and backdrop-material preparation behind extracted render-view and
      scene-contribution resources in the `render` subapp
- [x] Move desktop surface ordering, cursor view selection, and scene-contribution generation onto
      render-world snapshots consumed by the `render` subapp
- [x] Feed shell-owned pointer, cursor, surface-presentation, and overlay snapshots into the
      `render subapp` through `ShellRenderInput` instead of direct per-resource extraction clones
- [x] Feed pending screenshot/readback requests into the `render subapp` through
      `ShellRenderInput` instead of a direct resource clone
- [x] Let cursor-scene emission and readback phase planning consume `ShellRenderInput` directly
      instead of rehydrating separate render-world cursor/readback compatibility resources first
- [x] Let cursor-scene snapshotting and output-overlay scene synchronization consume
      `ShellRenderInput` directly instead of rehydrating separate render-world pointer/overlay
      compatibility resources first
- [x] Let frame-callback selection and damage tracking consume
      `ShellRenderInput.surface_presentation` directly instead of depending on a separate
      render-world surface-presentation compatibility resource first
- [x] Let desktop scene contribution assembly and surface-bridge preparation consume
      `ShellRenderInput.surface_presentation` directly instead of depending on a separate
      render-world surface-presentation compatibility resource first
- [x] Stop passing `SurfacePresentationSnapshot` as a normal runtime input into desktop scene
      contribution assembly; keep that path mailbox-first on
      `ShellRenderInput.surface_presentation`
- [x] Stop passing `SurfacePresentationSnapshot` as a normal runtime input into surface-process
      snapshot extraction; keep that path mailbox-first on
      `ShellRenderInput.surface_presentation`
- [x] Stop passing `SurfacePresentationSnapshot` as a normal runtime input into legacy desktop
      surface ordering and composition helpers; keep those paths mailbox-first on
      `ShellRenderInput.surface_presentation`
- [x] Stop passing `SurfacePresentationSnapshot` as a normal runtime input into frame-callback and
      surface-bridge preparation; keep those paths mailbox-first on
      `ShellRenderInput.surface_presentation`
- [x] Stop passing `SurfacePresentationSnapshot` as a normal runtime input into damage tracking;
      keep that path mailbox-first on `ShellRenderInput.surface_presentation`
- [x] Stop default-initializing and mailbox-rehydrating `SurfacePresentationSnapshot` inside the
      `render subapp` now that the normal path consumes `ShellRenderInput.surface_presentation`
      directly
- [x] Feed platform-owned output and surface snapshots into the `render subapp` from
      `WaylandIngress` before falling back to direct main-world compatibility resources
- [x] Make render extract consume mailbox-owned output, surface, and presentation snapshots for
      view, desktop-order, content-version, attachment, and scene-process extraction paths
- [x] Make direct scene-process extraction prefer `WaylandIngress.output_snapshots` over local
      output snapshot compatibility resources in the main-world legacy render path
- [x] Stop initializing main-world render compat copies of platform feedback/surface snapshots
      where normal render extract already consumes mailbox-owned data
- [x] Remove direct `WaylandFeedback -> render` extraction where render only needed debug-side
      screenshot bookkeeping
- [x] Build render-view, surface-content-version, and surface-buffer snapshots directly during
      `render subapp` extract instead of synchronizing them through main-world render systems
- [x] Build desktop surface ordering directly during `render subapp` extract instead of keeping a
      main-world render ordering system alive
- [x] Move generic blur/shadow/rounded-corner material request generation into the
      `render subapp` prepare phase using extracted render-view snapshots
- [x] Move render graph/material projection/process-plan compilation into a `render` subapp and
      sync the compiled outputs back to `main app`
- [x] Move damage tracking, frame callback selection, and presentation-feedback bookkeeping into
      the `render` subapp and sync the results back to `main app`
- [x] Make present-time executors consume compiled per-output GPU preparation state for surface
      import availability instead of relying only on scene-item-local readiness flags
- [x] Carry per-output surface import strategies through compiled GPU prep so present-time
      executors can reject unsupported imports from prepared state
- [x] Carry per-output process-shader requirements through compiled GPU prep so present-time
      prewarm consumes prepared state instead of scanning raw process plans first
- [x] Preallocate output execution targets from compiled target-allocation plans before graph
      execution instead of lazily creating them inside pass execution
- [x] Shrink the wayland/render boundary so present code consumes compiled frame outputs instead of
      render-internal planning resources directly
- [x] Carry output-local readback request plans inside `CompiledOutputFrame` and have present-time
      executors consume those plans directly
- [x] Export explicit per-output final present targets inside `CompiledOutputFrames`
- [x] Keep the `render subapp` compositor-oriented while making it capable of hosting a stronger
      GPU implementation later

### Milestone 4: Render Runtime Upgrade

Status: in_progress

Goal: upgrade render internals from the current hard-coded effect adapter model toward a stronger
typed material, pipeline specialization, phase, and graph architecture.

Depends on: Milestone 3

Exit criteria:

- render internals no longer rely on a central effect enum or string-keyed parameter maps as the
  primary extension model
- typed material, pipeline specialization, render phase, and render graph abstractions are the
  target architecture
- adding a new compositor render feature no longer requires editing a central render-core
  switchboard

Planning docs:

- [Render SubApp Architecture TODO](./milestones/render-subapp-architecture.md)

- [x] Replace hard-coded effect enums and string-keyed parameter maps with typed material and
      pipeline abstractions
- [x] Introduce render-world stages aligned with
      `extract -> prepare -> queue -> execute -> cleanup`
- [x] Let typed material definitions carry shader source, bind-group layout key, specialization
      key, and queue metadata into compiled frame state
- [x] Replace ad-hoc string pipeline keys with structured material specialization keys
- [x] Introduce generic render phase planning ahead of graph compilation for scene/post-process/
      readback work
- [x] Add render-world pipeline cache state for scene draws and post-process passes
- [x] Build stable per-output `CompiledOutputFrame` entries and have present paths consume them
- [x] Track output format, sample mode, blend mode, clipping mode, and pass role in specialization
      keys
- [x] Add render-world target allocation and surface-texture bridge planning resources
- [x] Add descriptor-level prepared GPU resource caches for output targets, surface imports, and
      material bindings so compiled frames carry explicit preparation state ahead of backend
      execution
- [x] Carry stable platform surface kinds through the surface-texture bridge plan for render-side
      import preparation
- [x] Carry stable platform buffer-source metadata through surface snapshots and texture-bridge
      descriptors so SHM-backed imports have an explicit contract
- [x] Export platform import capabilities through `WaylandIngress` and gate prepared dma-buf
      import strategy on mailbox-owned capability snapshots
- [x] Move SHM/dma-buf import-strategy selection behind wayland-owned platform snapshots instead of
      deriving it inside render prepare
- [x] Implement actual dma-buf surface import in the render/backend runtime cache instead of only
      carrying capability and strategy descriptors through prepared state
- [x] Add external texture import support, wiring it through platform snapshots, prepared GPU
      caches, and present-time execution on supported backends
- [x] Prepare surface, cursor, solid-rect, backdrop, and overlay scene items through a unified
      render-world scene resource cache ahead of backend execution
- [x] Move compositor effects toward independent render feature plugins
- [x] Drive screenshot/readback execution from readback graph nodes and pass payloads instead of a
      separate executor-side global readback lookup
