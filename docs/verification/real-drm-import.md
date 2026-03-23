# Real DRM Import Verification

This document describes the hardware-backed validation path for non-SHM imports after the boundary
and subapp migration.

## Scope

The verification path targets:

- dma-buf surface import on a real DRM/GBM-capable backend
- external-texture classification for dma-buf formats that are importable but not directly
  renderable
- runtime failure reporting when import or present fails even though the backend advertises
  non-SHM import capability

## Opt-In Test

The repository now ships an opt-in integration test:

- [real_drm_dmabuf_import.rs](/home/misaka/Code/nekoland/nekoland/tests/real_drm_dmabuf_import.rs)

It is intentionally disabled by default so normal CI and developer `cargo test` runs do not
require real DRM session access.

### Run

```bash
NEKOLAND_BACKEND=drm \
NEKOLAND_RUN_REAL_DRM_IMPORT_TEST=1 \
cargo test -p nekoland --test real_drm_dmabuf_import -- --exact real_drm_backend_imports_dmabuf_surface_end_to_end
```

Optional override:

```bash
NEKOLAND_TEST_RENDER_NODE=/dev/dri/renderD128
```

If `NEKOLAND_TEST_RENDER_NODE` is unset, the test scans `/dev/dri` for a `renderD*` node.

## What The Test Verifies

The helper client allocates a real GBM buffer, exports it as a dma-buf, and submits it through
`zwp_linux_dmabuf_v1`. After the compositor run completes, the test asserts that:

1. `WaylandIngress.surface_snapshots` marks the surface as `DmaBuf`
2. the platform snapshot exports dma-buf format metadata and a non-`Unsupported` import strategy
3. `CompiledOutputFrames` carries a prepared import descriptor for the same stable `surface_id`
4. `WaylandFeedback.present_audit` shows that the surface actually entered the present path

This is the repository's real-hardware validation path for non-SHM imports.

## Failure Reporting

When a backend advertises non-SHM import capability but runtime import/present still fails, the
backend now records structured diagnostics in:

- [PlatformImportDiagnosticsState](/home/misaka/Code/nekoland/crates/nekoland-ecs/src/resources/platform_backend.rs)
- [WaylandFeedback.import_diagnostics](/home/misaka/Code/nekoland/crates/nekoland-ecs/src/resources/app_boundary.rs)

Each diagnostic includes:

- `output_name`
- optional `surface_id`
- optional `strategy`
- failure `stage` (`surface_import` or `present`)
- human-readable `message`

This is intended to catch cases where:

- the backend reports dma-buf import capability
- the surface is classified as `DmaBufImport` or `ExternalTextureImport`
- but `import_surface_tree(...)` or the later present step still fails at runtime

## Interpreting Results

- `skip`: the host does not provide the required DRM session/render-node capability
- `pass`: the dma-buf buffer made it through wayland snapshots, render prepared imports, and
  backend present audit
- `fail`: inspect `WaylandFeedback.import_diagnostics` and the test stderr to determine whether the
  failure happened during surface import or later present execution
