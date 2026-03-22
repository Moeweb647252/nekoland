# Post-Migration TODO

This roadmap starts after `docs/TODO.md` and the milestone architecture docs are complete.

## Goal

Turn the finished mailbox/subapp migration into a better-validated and more production-ready
runtime without reopening the architectural boundary work that has already landed.

## Work Items

### 1. Real Backend Verification

- [x] Add end-to-end verification for dma-buf / external-texture import on real DRM/GBM-capable
      backends instead of relying only on unit tests and nested/virtual smoke paths
- [x] Capture and document at least one real-hardware validation path for non-SHM imports,
      including failure reporting when a backend advertises import capability but import/present
      still fails at runtime

### 2. GPU Runtime Object Caches

- [ ] Extend the current prepared target/shader/surface-import caches into a clearer runtime GPU
      object cache model for textures, buffers, and bind groups
- [ ] Key GPU object reuse explicitly by output/material/surface content version so present-time
      execution does not need to rebuild transient objects beyond actual invalidation points

### 3. Crate And Module Boundary Cleanup

- [ ] Revisit crate/module boundaries now that the migration is complete, especially around the
      dispersed `wayland` runtime slices in `protocol` / `backend` / `input`
- [ ] Split oversized plugin/orchestration modules into clearer runtime slices such as bootstrap,
      extract, normalize/apply, present/feedback, render extract, render prepare, and render
      execute

### 4. Broader Integration Coverage

- [ ] Add heavier integration and soak coverage for long-running sessions, multi-output
      presentation, and backend combinations beyond the current smoke/test matrix
- [ ] Add targeted regression coverage for non-SHM import paths so external-texture / dma-buf
      behavior stays exercised after future backend/render refactors
