#!/usr/bin/env bash
set -euo pipefail

echo "Checking nekoland workspace..."
cargo fmt --all --check
cargo check --workspace

