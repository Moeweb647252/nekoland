# nekoland

`nekoland` is a multi-crate Bevy ECS and Smithay based Wayland compositor workspace.

The repository is no longer just a compile-time scaffold. It contains a working compositor
architecture with:

- a boundary-driven main loop
- a dedicated Wayland/platform sub-app
- a dedicated render sub-app
- nested `winit`, physical `drm`, and offscreen `virtual` backends
- XDG shell, layer-shell, selection, presentation-time, and XWayland paths
- an IPC control plane with a companion CLI
- broad in-process, integration, IPC, virtual-output, and backend-oriented tests

## Architecture

The compositor is organized around three explicit worlds:

- `main app`: authoritative shell state and policy
- `wayland subapp`: Smithay protocol runtime, backend runtime, output lifecycle, input extraction,
  and present-time feedback
- `render subapp`: render-world extraction, render graph compilation, resource preparation, damage
  tracking, and `CompiledOutputFrames`

The root frame loop runs in this order:

1. `wayland extract`
2. `main input/layout`
3. `render`
4. `wayland present`

Cross-world communication is done through explicit resources rather than direct world access:

- `WaylandIngress`
- `ShellRenderInput`
- `WaylandCommands`
- `CompiledOutputFrames`
- `WaylandFeedback`

That split is the central design choice of the codebase.

## Workspace

- `crates/nekoland-core`: app lifecycle, schedules, sub-app orchestration, calloop integration
- `crates/nekoland-config`: config schema, loading, normalization, hot reload
- `crates/nekoland-ecs`: shared ECS data model, components, resources, views, control helpers
- `crates/nekoland-input`: normalized input decoding, keybindings, seat management
- `crates/nekoland-shell`: workspaces, focus, layout, popups, layer-shell, window lifecycle
- `crates/nekoland-protocol`: Smithay protocol runtime, surface tracking, XWayland, feedback
- `crates/nekoland-render`: render extraction, scene assembly, materials, graph compilation,
  resource preparation, damage, readback
- `crates/nekoland-backend`: backend abstraction and runtime implementations for `winit`, `drm`,
  and `virtual`
- `crates/nekoland-ipc`: IPC server, query cache, subscriptions, command model
- `nekoland`: compositor binary
- `nekoland-msg`: IPC client CLI

## Current State

Implemented today:

- nested development path via `winit`
- DRM backend path with libseat/libinput/GBM plumbing
- virtual output backend for capture/debug/testing
- output discovery and ECS materialization
- XDG toplevel and popup lifecycle handling
- layer-shell surface lifecycle and work-area calculation
- window focus, stacking, tiling, floating, maximize, fullscreen, background-window policy
- typed render-material system and render-graph compilation
- damage tracking, frame callbacks, presentation feedback, screenshot/readback plumbing
- clipboard, drag-and-drop, primary selection, keyboard-layout switching
- XWayland runtime and X11 window mapping/reconfiguration paths
- IPC query/action/subscribe control plane

This is still an in-progress compositor, but the repository should be understood as an active
implementation, not as a placeholder scaffold.

## Running

Default nested run:

```bash
cargo run -p nekoland
```

Equivalent helper script:

```bash
bash ./tools/run-nested.sh
```

Run with the DRM backend:

```bash
NEKOLAND_BACKEND=drm cargo run -p nekoland
```

Equivalent helper script:

```bash
bash ./tools/run-drm.sh
```

Useful runtime environment variables:

- `NEKOLAND_CONFIG`: override config path, default `config/default.toml`
- `NEKOLAND_BACKEND`: comma-separated backend list, default `winit`
- `NEKOLAND_SEAT`: DRM seat name, default `seat0`
- `NEKOLAND_RUNTIME_DIR`: runtime dir override used by Wayland socket bootstrap and IPC
- `NEKOLAND_IPC_SOCKET`: explicit IPC socket path override
- `NEKOLAND_MAX_FRAMES`: optional frame cap for test/dev runs
- `NEKOLAND_FRAME_TIMEOUT_MS`: outer loop frame timeout
- `NEKOLAND_DISABLE_STARTUP_COMMANDS`: disables configured startup actions when set to a non-empty
  value other than `0` or `false`

Logging is controlled through `RUST_LOG`. A useful default is:

```bash
RUST_LOG=info,nekoland=debug cargo run -p nekoland
```

## Configuration

The default config lives at:

- `config/default.toml`

Additional examples:

- `config/example.toml`
- `config/full-example.toml`

The config surface currently covers:

- theme metadata and colors
- default window layout policy
- keyboard repeat and keyboard layouts
- startup actions
- XWayland enable/disable
- output definitions
- window rules
- keybindings
- IPC command-history limits

Config hot reload is enabled. You can also force a reload through IPC:

```bash
cargo run -p nekoland-msg -- action reload-config
```

For a fuller inventory of runtime configuration and environment variables, see:

- `docs/config-inventory.md`

## IPC And CLI

`nekoland-msg` is the control-plane client for the compositor IPC socket.

Typical queries:

```bash
cargo run -p nekoland-msg -- query tree
cargo run -p nekoland-msg -- query outputs
cargo run -p nekoland-msg -- query present-audit
```

Typical actions:

```bash
cargo run -p nekoland-msg -- action spawn foot
cargo run -p nekoland-msg -- action quit
cargo run -p nekoland-msg -- window close 42
cargo run -p nekoland-msg -- workspace switch 2
```

Subscriptions:

```bash
cargo run -p nekoland-msg -- subscribe window
cargo run -p nekoland-msg -- subscribe focus --event focus_changed
cargo run -p nekoland-msg -- subscribe all --jsonl --no-payloads
```

Supported subscription topics currently include:

- `window`
- `popup`
- `workspace`
- `output`
- `command`
- `config`
- `keyboard-layout`
- `clipboard`
- `primary-selection`
- `present-audit`
- `focus`
- `tree`
- `all`

The IPC socket path resolves in this order:

1. `NEKOLAND_IPC_SOCKET`
2. `${NEKOLAND_RUNTIME_DIR}/nekoland-ipc.sock`
3. `${XDG_RUNTIME_DIR}/nekoland-ipc.sock`
4. `/tmp/nekoland-ipc.sock`

## Protocol Surface

The compositor currently advertises these Wayland globals:

- `wl_compositor`
- `wl_subcompositor`
- `xdg_wm_base`
- `ext_foreign_toplevel_list_v1`
- `xdg_activation_v1`
- `zxdg_decoration_manager_v1`
- `zwlr_layer_shell_v1`
- `wl_data_device_manager`
- `zwp_primary_selection_device_manager_v1`
- `zwp_linux_dmabuf_v1`
- `wp_viewporter`
- `wp_fractional_scale_manager_v1`
- `wl_shm`
- `wl_seat`
- `wl_output`
- `zxdg_output_manager_v1`
- `wp_presentation`

XWayland support is also wired through the protocol/runtime layer and can be disabled in config.

## Testing And Verification

Run the standard workspace checks:

```bash
bash ./tools/dev-env.sh
```

Run all tests:

```bash
cargo test --workspace
```

The test suite includes:

- unit tests across crates
- integration tests under `tests/`
- in-process compositor tests under `nekoland/tests/`
- IPC and subscription tests
- virtual-output and screenshot/readback tests
- XWayland smoke coverage
- DRM/dmabuf-oriented tests
- WLCS protocol conformance scaffolding

## Shell Completion

Generate `nekoland-msg` shell completions with:

```bash
bash ./tools/generate-completions.sh
```

This writes:

- `completions/nekoland-msg.bash`
- `completions/_nekoland-msg`
- `completions/nekoland-msg.fish`

You can also print a single shell directly:

```bash
cargo run -p nekoland-msg -- completion bash
```

Verify the checked-in completions are current:

```bash
bash ./tools/generate-completions.sh --check
```
