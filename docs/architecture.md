# Architecture

`nekoland` follows a workspace-first layout:

- `nekoland-ecs` is the pure data model and does not depend on Smithay.
- `nekoland-protocol` converts protocol-oriented state into ECS-friendly queues.
- feature crates (`input`, `shell`, `render`, `backend`, `ipc`, `config`) register systems through `nekoland-core`.
- the `nekoland` binary assembles plugins and owns startup.

This repository currently implements the boundaries and placeholders required to grow into the full compositor.

Additional design notes:

- [`control_plane.md`](control_plane.md) documents the high-level control-plane boundary for
  window/workspace/output actions.
