# nekoland

`nekoland` is a multi-crate Bevy ECS driven Wayland compositor workspace scaffold.

## Workspace

- `crates/nekoland-core`: app lifecycle, schedules, plugin contract, bridge primitives.
- `crates/nekoland-protocol`: Wayland protocol state wrappers and registry.
- `crates/nekoland-ecs`: pure ECS data model.
- `crates/nekoland-input`: input extraction and keybinding pipeline.
- `crates/nekoland-shell`: window management, focus, layout, workspaces.
- `crates/nekoland-render`: render graph, damage tracking, effects, frame callbacks.
- `crates/nekoland-backend`: backend abstraction for DRM, Winit, X11.
- `crates/nekoland-ipc`: socket protocol and command model.
- `crates/nekoland-config`: config schema, loading, hot reload, theme parsing.
- `nekoland`: main compositor binary.
- `nekoland-msg`: IPC client CLI.

## Status

This repository currently provides an engineering scaffold with compile-oriented module boundaries and placeholder systems. The next step is filling each crate with concrete Smithay, backend, and renderer integrations.

## Shell Completion

Generate `nekoland-msg` shell completions with:

```bash
bash ./tools/generate-completions.sh
```

This writes the current `clap`-derived completions to:

- `completions/nekoland-msg.bash`
- `completions/_nekoland-msg`
- `completions/nekoland-msg.fish`

You can also print one shell directly:

```bash
cargo run -p nekoland-msg -- completion bash
```

Verify the checked-in completions are current with:

```bash
bash ./tools/generate-completions.sh --check
```
