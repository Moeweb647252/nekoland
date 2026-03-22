# Shell App Architecture TODO

Parent milestone: [Main App / Shell Authority](../TODO.md)

## Goal

Build `main app` as the authoritative shell world of the compositor. The shell app should own
focus, workspace, layout, window/layer/popup policy, and semantic input handling while depending
on explicit mailbox resources instead of direct protocol/backend/render coupling.

## Architecture Constraints

- [x] `main app` owns shell policy and authoritative shell state
- [x] `main app` does not own protocol objects, backend handles, or present runtime state
- [x] `main app` does not own render-world GPU resources or render executor internals
- [x] Cross-app communication uses explicit mailbox resources only
- [x] Shell-facing state that must cross app boundaries uses stable ids and immutable frame
      snapshots instead of cross-world `Entity` values

## Cross-App Mailboxes

- [x] Consume `WaylandIngress` as the only platform-to-shell input mailbox
- [x] Produce `ShellRenderInput` as the only shell-to-render mailbox
- [x] Produce `WaylandCommands` as the only shell-to-platform command mailbox
- [x] Consume `WaylandFeedback` where shell state needs present-time results or protocol feedback
- [x] Keep mailbox ownership explicit so `main app`, `wayland`, and `render` do not reach into
      each other's private world state

## Work Items

### 1. Main App And Root Runner Role

- [x] Define `main app` as the source of truth for shell state and policy
- [x] Keep the root runner outside the subapps and make its order explicit
- [x] Ensure `main app` runs between `wayland` extraction and `render` update
- [x] Keep orchestration policy in the root runner instead of scattering it through plugin startup

### 2. Shell-Owned State And Stable Ids

- [x] Audit shell-owned state and keep focus, workspace, layout, window, layer, popup, and
      semantic input policy in `main app`
- [x] Define or reuse stable ids for outputs, surfaces, windows, layers, popups, and workspaces
      where cross-app references are required
- [x] Remove assumptions that shell code can hand raw `Entity` values across app boundaries
- [x] Keep shell state authoritative even when platform/render snapshots are derived from it

### 3. Input Normalization Boundary

- [x] Consume normalized platform input from `WaylandIngress`
- [x] Keep low-level seat/input extraction in the `wayland` subapp
- [x] Keep high-level bindings, routing policy, and semantic actions in `main app`
- [x] Ensure shell input systems no longer depend on backend/protocol runtime resources directly

### 4. Shell Policy Systems

- [x] Keep focus, stacking, tiling, floating, fullscreen, layer arrangement, popup policy, and
      workspace policy in `main app`
- [x] Keep shell lifecycle policy authoritative for window/layer/popup state machines
- [x] Ensure output/workspace policy depends on normalized snapshots rather than backend runtime
      handles
- [x] Keep shell-level animation and presentation intent in `main app` until extracted into render

### 5. Shell-To-Render Boundary

- [x] Define `ShellRenderInput` as the only shell-to-render data path
- [x] Extract shell-owned scene and presentation state into immutable frame snapshots
- [x] Separate shell policy/state mutation from render-facing extraction
- [x] Ensure render consumes shell snapshots instead of reading shell-world internals directly

### 6. Shell-To-Wayland Boundary

- [x] Define `WaylandCommands` as the only shell-to-platform command path
- [x] Read nested-session startup/command-launch environment from `WaylandIngress` instead of
      direct protocol server resources
- [x] Keep shell-owned output command and overlay state initialized in `ShellPlugin` before it is
      mirrored into `WaylandCommands`
- [x] Route configure requests, popup grabs, focus handoff, seat changes, output control, and
      protocol-facing shell decisions through explicit commands
- [x] Remove direct shell access to protocol/backend resources where possible
- [x] Keep platform side effects owned by `wayland` even when the decision originates in shell

### 7. Decoupling Existing Code

- [x] Remove direct shell/protocol/backend coupling where shell systems currently reach into
      non-shell resources
- [x] Reduce shell/render coupling so shell systems do not depend on render-internal resources
- [x] Keep shared pure ECS data structures in common crates instead of duplicating shell-only
      copies
- [x] Make cross-app state exchange obvious enough that another engineer can trace ownership
      without reading startup order code

### 8. Verification

- [x] Add tests that verify shell policy still works when platform and render are treated as
      mailbox-driven boundaries
- [x] Add tests for stable-id usage across shell/platform/render boundaries
- [x] Add at least one smoke path that exercises:
      `wayland poll -> main shell update -> render update -> wayland present`
