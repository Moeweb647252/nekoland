# Repository Guidelines

## Project Structure & Module Organization
`nekoland` is a Rust workspace. Core crates live under `crates/`: `nekoland-core` owns app lifecycle and plugin wiring, `nekoland-ecs` holds the pure ECS model, and feature crates such as `nekoland-input`, `nekoland-shell`, `nekoland-render`, `nekoland-backend`, `nekoland-ipc`, and `nekoland-config` add behavior. The compositor binary is in `nekoland/`; the IPC CLI is in `nekoland-msg/`. Integration coverage is split across `tests/integration/`, `nekoland/tests/`, and `nekoland-msg/tests/`. Runtime examples live in `config/`, reference docs in `docs/`, assets in `assets/`, and helper scripts in `tools/`.

Architecture notes worth checking before structural changes:

- `docs/architecture.md`
- `docs/control_plane.md`
- `agent/current_plan.md` for active follow-up work
- `agent/completed_plan.md` for recently finished migrations and their intended end state

## Build, Test, and Development Commands
Use Cargo from the repository root.

- `cargo check --workspace`: fast workspace validation; also used by `bash ./tools/dev-env.sh`.
- `cargo test --workspace`: run unit, integration, and end-to-end tests.
- `cargo test -p nekoland-msg`: run the CLI tests used in CI.
- `cargo fmt --all --check`: enforce formatting before review.
- `cargo clippy --workspace --all-targets`: catch lint violations; `dbg!`, `todo!`, and `unwrap()` are denied.
- `bash ./tools/run-nested.sh`: launch the compositor with `config/default.toml` in nested `winit` mode.
- `bash ./tools/generate-completions.sh --check`: verify checked-in shell completions under `completions/`.

## Coding Style & Naming Conventions
Follow standard Rust style: 4-space indentation, `snake_case` for files/modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Keep crate boundaries explicit and place platform-agnostic data in `nekoland-ecs` instead of Smithay-facing crates. Format with `rustfmt` and prefer small, composable systems over monolithic modules.

Control-plane rules:

- Treat `PendingWindowControls`, `PendingWorkspaceControls`, `PendingOutputControls` and their facades (`WindowOps`, `WorkspaceOps`, `OutputOps`) as the public control surface for user-facing actions.
- Do not introduce new user-facing flows that write `PendingWindowServerRequests` or `PendingOutputServerRequests` directly; those queues are internal bridge layers.
- Parse strings and bare numbers at the boundary (IPC/config/keybindings), then convert to typed selectors or typed IDs immediately.
- Prefer typed selector/id types from `nekoland-ecs::selectors` over raw `String` / `u64`.
- For existing runtime objects, prefer ECS query semantics and explicit markers such as `ActiveWorkspace` instead of string lookup or boolean flags alone when that makes the query shape clearer.

Backend rules:

- Keep backend work inside the manager/runtime architecture; do not reintroduce global backend-kind gating patterns.
- Backend instances are full input+output runtime units. Avoid splitting new backend behavior into disconnected global systems when it belongs inside a backend runtime.
- Output ownership should remain explicit through backend ownership metadata instead of implicit global selection.

## Testing Guidelines
Add tests next to the behavior they cover. Use `*_subscription.rs`, `ipc_*.rs`, or similarly descriptive `snake_case` names that state the scenario, mirroring the existing suite. Favor focused integration tests in `tests/integration/` for cross-crate behavior and binary-specific tests in `nekoland/tests/` or `nekoland-msg/tests/`. Run `cargo test --workspace` before opening a PR.

## Commit & Pull Request Guidelines
Current history uses Conventional Commit prefixes such as `feat:`. Continue with short, imperative subjects like `fix: validate output scale on reload`. PRs should explain the affected crate(s), describe user-visible behavior, link related issues, and include logs or screenshots when changing CLI output, rendering, or generated completions. If completions change, commit the regenerated files in `completions/`.
