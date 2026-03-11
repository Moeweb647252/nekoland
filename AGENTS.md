# Repository Guidelines

## Project Structure & Module Organization
`nekoland` is a Rust workspace. Core crates live under `crates/`: `nekoland-core` owns app lifecycle and plugin wiring, `nekoland-ecs` holds the pure ECS model, and feature crates such as `nekoland-input`, `nekoland-shell`, `nekoland-render`, `nekoland-backend`, `nekoland-ipc`, and `nekoland-config` add behavior. The compositor binary is in `nekoland/`; the IPC CLI is in `nekoland-msg/`. Integration coverage is split across `tests/integration/`, `nekoland/tests/`, and `nekoland-msg/tests/`. Runtime examples live in `config/`, reference docs in `docs/`, assets in `assets/`, and helper scripts in `tools/`.

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

## Testing Guidelines
Add tests next to the behavior they cover. Use `*_subscription.rs`, `ipc_*.rs`, or similarly descriptive `snake_case` names that state the scenario, mirroring the existing suite. Favor focused integration tests in `tests/integration/` for cross-crate behavior and binary-specific tests in `nekoland/tests/` or `nekoland-msg/tests/`. Run `cargo test --workspace` before opening a PR.

## Commit & Pull Request Guidelines
Current history uses Conventional Commit prefixes such as `feat:`. Continue with short, imperative subjects like `fix: validate output scale on reload`. PRs should explain the affected crate(s), describe user-visible behavior, link related issues, and include logs or screenshots when changing CLI output, rendering, or generated completions. If completions change, commit the regenerated files in `completions/`.
