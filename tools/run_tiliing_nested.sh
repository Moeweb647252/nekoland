#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "${SCRIPT_DIR}/.." && pwd)

BASE_CONFIG=""
for candidate in \
    "${REPO_ROOT}/config/tiling.toml" \
    "${REPO_ROOT}/config/full-example.toml" \
    "${REPO_ROOT}/config/example.toml" \
    "${REPO_ROOT}/config/default.toml"
do
    if [[ -f "${candidate}" ]]; then
        BASE_CONFIG="${candidate}"
        break
    fi
done

if [[ -z "${BASE_CONFIG}" ]]; then
    echo "failed to find a base config under ${REPO_ROOT}/config" >&2
    exit 1
fi

TEMP_DIR=$(mktemp -d -t nekoland-tiliing-nested.XXXXXX)
trap 'rm -rf -- "${TEMP_DIR}"' EXIT

CONFIG_PATH="${TEMP_DIR}/tiling-nested.toml"

if grep -q '^default_layout[[:space:]]*=' "${BASE_CONFIG}"; then
    sed 's/^default_layout[[:space:]]*=.*/default_layout = "tiling"/' "${BASE_CONFIG}" > "${CONFIG_PATH}"
else
    {
        printf 'default_layout = "tiling"\n\n'
        cat "${BASE_CONFIG}"
    } > "${CONFIG_PATH}"
fi

cd -- "${REPO_ROOT}"

export NEKOLAND_CONFIG="${CONFIG_PATH}"
export NEKOLAND_BACKEND="${NEKOLAND_BACKEND:-winit}"

echo "repo root: ${REPO_ROOT}"
echo "base config: ${BASE_CONFIG}"
echo "temp config: ${CONFIG_PATH}"
echo "backend: ${NEKOLAND_BACKEND}"
echo "launching nested nekoland with tiling defaults"

exec cargo run -p nekoland -- "$@"
