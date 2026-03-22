# Render SubApp Architecture TODO

Parent milestones:

- [Render SubApp Boundary](../TODO.md)
- [Render Runtime Upgrade](../TODO.md)

## Goal

Build a real render runtime for the compositor instead of a thin effect adapter layer. The render
subapp should consume shell-owned frame snapshots, maintain its own render world, and produce
`CompiledOutputFrames` for the wayland subapp to present.

## Architecture Constraints

- [x] `render` does not own `wl_surface`, DRM, or winit objects
- [x] `render` does not consume cross-world `Entity` values
- [x] Cross-app inputs are stable ids plus immutable frame snapshots
- [x] Output of the render subapp is a compiled per-output frame description, not direct protocol
      side effects
- [x] Internal stages follow `extract -> prepare -> queue -> execute -> cleanup`

## Work Items

### 1. Render App Skeleton

- [x] Define the `render` subapp label and plugin entrypoint
- [x] Add render-world resources for extracted views, scene primitives, material instances, and
      compiled frame outputs
- [x] Add an explicit runner/update entry so the root app controls when the render subapp runs
- [x] Keep the shell world as source-of-truth; only extracted render snapshots enter the render
      world

### 2. Cross-App Inputs And Outputs

- [x] Define `ShellRenderInput` as the only shell-to-render mailbox
- [x] Define stable render-facing ids for outputs, surfaces, windows, layers, cursors, and scene
      items where needed
- [x] Define `CompiledOutputFrame` / `CompiledOutputFrames` as the only render-to-wayland mailbox
- [x] Ensure readback requests/results and damage metadata travel through explicit mailbox
      resources instead of hidden side channels

### 3. Render Scene Extraction

- [x] Extract output views, cursor state, overlays, surface presentation snapshots, and compositor
      primitives into render-world data
- [x] Convert shell-facing scene snapshots into render primitives with stable sort keys and draw
      ordering
- [x] Keep extraction deterministic and frame-local
- [x] Separate extraction of shell scene data from GPU resource preparation
- [x] Feed shell-owned pointer, cursor, surface-presentation, and overlay snapshots through the
      `ShellRenderInput` mailbox instead of direct per-resource render extract clones
- [x] Feed pending screenshot/readback requests through the `ShellRenderInput` mailbox instead of
      a direct render extract clone
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
- [x] Prefer platform-owned output and surface snapshots from `WaylandIngress` during render
      extract instead of direct main-world compatibility resources where the mailbox already
      carries the same data
- [x] Consume mailbox-owned output, surface, and presentation snapshots for render-view,
      desktop-order, content-version, attachment, and scene-process extract paths
- [x] Prefer `WaylandIngress.output_snapshots` even in the main-world legacy scene-process path
      instead of local output snapshot compatibility resources first
- [x] Stop initializing main-world render compat copies of platform feedback/surface snapshots
      where normal render extract already consumes mailbox-owned data
- [x] Remove direct `WaylandFeedback -> render` extraction where render only needed debug-side
      screenshot bookkeeping
- [x] Build simple render-world snapshots such as output views, surface content versions, and
      buffer attachments directly during subapp extract instead of routing them through main-world
      render systems
- [x] Derive surface buffer-attachment readiness from platform surface snapshots during render
      extract instead of direct main-world buffer-state queries
- [x] Derive surface content versions from platform surface snapshots during render extract/bridge
      preparation instead of direct main-world content-version queries
- [x] Build desktop surface ordering directly during subapp extract instead of depending on a
      main-world render ordering system

### 4. Typed Material System

- [x] Remove the central `RenderMaterialParams` enum as the primary material model
- [x] Replace string-keyed uniform maps with typed parameter structs
- [x] Introduce a typed material registration model similar in spirit to `ShaderType` /
      `AsBindGroup`
- [x] Let each material/effect define:
      shader source, bind-group layout, typed params, specialization key, and queue logic
- [x] Keep material registration open-ended so new compositor effects do not require editing a
      global enum
- [x] Move compositor effects toward independent render feature plugins so each feature owns its
      extract/prepare/queue/graph integration instead of expanding render-core switch statements
- [x] Emit generic blur/shadow/rounded-corner material requests from render-world view snapshots in
      the render subapp instead of main-world pre-render systems

### 5. Pipeline And Shader Compilation

- [x] Introduce `PipelineKey` / `SpecializedPipelineKey` types instead of ad-hoc string shader keys
- [x] Add pipeline caches for scene draws and post-process passes
- [x] Track output format, sample mode, blend mode, clipping mode, and pass role in specialization
      keys
- [x] Compile shaders and pipeline state during `prepare`/`queue`, not ad hoc during execution

### 6. Render Phases And Graph

- [x] Replace effect-specific queueing with generic render phases or pass item lists
- [x] Keep a render graph, but make graph nodes consume typed pass inputs instead of hard-coded
      effect names
- [x] Support scene passes, post-process passes, composite passes, and readback passes as generic
      node categories
- [x] Make output-local graph compilation produce a stable `CompiledOutputFrame`

### 7. GPU Resource Preparation

- [x] Introduce prepared textures, buffers, bind groups, and intermediate targets in render-world
      caches
- [x] Add descriptor-level prepared GPU resource caches for output targets, surface imports, and
      material bindings so compiled frames carry output-local preparation state before backend
      execution
- [x] Carry per-output process-shader requirements through prepared GPU state so present-time
      prewarm consumes prepared descriptors instead of rescanning raw process plans first
- [x] Preallocate output execution targets from compiled target-allocation plans before graph
      execution instead of lazily creating them during pass execution
- [x] Make present-time executors consume compiled per-output surface-import preparation state
      instead of relying only on scene-item-local readiness flags
- [x] Carry per-output surface import strategies through compiled GPU preparation state so
      present-time executors can reject unsupported imports from prepared data
- [x] Export platform import capabilities through `WaylandIngress` and gate prepared dma-buf
      import strategy on mailbox-owned capability snapshots
- [x] Move SHM/dma-buf import-strategy selection behind wayland-owned platform snapshots instead
      of deriving it inside render prepare
- [x] Promote prepared surface-import descriptors into actual runtime import caches keyed by
      surface/content version so dma-buf imports can be reused instead of only revalidated
- [x] Import dma-buf-backed surfaces as actual GPU/external textures on supported backends instead
      of stopping at strategy selection and preflight
- [x] Prepare imported surface textures through a dedicated abstraction instead of mixing that logic
      into shell or wayland code
- [x] Allocate and recycle output/intermediate targets per output view
- [x] Prepare cursor, solid-rect, overlay, and compositor-owned resources through the same cache
      model

### 8. Surface Texture Bridge

- [x] Define a render-facing surface texture import abstraction
- [x] Support shell scene items without exposing protocol/backend internals to the render subapp
- [x] Carry stable platform surface descriptors through the bridge plan instead of exposing live
      protocol objects
- [x] Start with a clear bridge contract for SHM-backed content
- [x] Extend the bridge to dma-buf/external textures only after the typed render pipeline is in
      place
- [x] Add explicit external-texture import descriptors and runtime handling for backends that can
      bind external textures without going through the SHM upload path

### 9. Execution And Readback

- [x] Execute compiled per-output frame compilation and render-side pacing bookkeeping in the
      render subapp
- [x] Produce explicit final output target plans as part of the compiled frame outputs exported to
      the wayland subapp
- [x] Carry output-local readback request plans inside compiled output frames so present-time
      executors no longer need a separate global readback lookup
- [x] Integrate screenshot/readback as graph nodes, not special-case backend code paths
- [x] Keep presentation submission outside the render subapp

### 10. Migration Of Existing Render Code

- [x] Move current `RenderPlan` extraction responsibilities into render extract stages where
      appropriate
- [x] Move desktop scene ordering, cursor view selection, and render-plan assembly behind render
      view / scene snapshot resources consumed by the subapp
- [x] Move render graph/material projection/process plan compilation behind the `render` subapp
      boundary and sync the results back to `main app`
- [x] Move damage tracking, frame callback selection, and presentation-feedback state behind the
      `render` subapp boundary and sync the results back to `main app`
- [x] Replace the current hard-coded effect adapters (`blur`, `shadow`, `rounded_corners`) with
      typed material/pass plugins
- [x] Keep render-core focused on shared infrastructure only:
      schedules, views, target allocation, graph compilation, pipeline caches, and surface texture
      import
- [x] Avoid turning render-core into a central effect registry that must be edited whenever a new
      feature is added
- [x] Retire `ProcessShaderKey(String)` in favor of typed pipeline specialization keys
- [x] Gradually reduce `gles_executor` so final present target selection comes from compiled render
      output plans instead of executor-side graph inference

### 11. Verification

- [x] Add tests for cross-app mailbox extraction and frame compilation
- [x] Add tests for typed material registration and specialization
- [x] Add tests for per-output graph compilation and readback
- [x] Add at least one smoke path that exercises `main -> render -> wayland present`
