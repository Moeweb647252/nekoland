# Plugin Guide

Implement `nekoland_core::plugin::NekolandPlugin` for any new feature crate.

Recommended pattern:

1. Initialize resources and events in `plugin.rs`.
2. Keep pure data in `nekoland-ecs`.
3. Register systems from focused modules such as `layout`, `effects`, or `commands`.
4. Keep Smithay or backend-specific glue out of `nekoland-ecs`.

