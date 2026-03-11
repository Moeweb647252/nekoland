#!/usr/bin/env bash
set -euo pipefail

echo "Starting nekoland in nested skeleton mode..."
exec env NEKOLAND_BACKEND="${NEKOLAND_BACKEND:-winit}" \
    NEKOLAND_CONFIG="${NEKOLAND_CONFIG:-config/default.toml}" cargo run -p nekoland -- "$@"
