#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
COMPLETION_DIR="${REPO_ROOT}/completions"
MODE="${1:-generate}"

generate_into() {
    local output_dir="$1"

    mkdir -p "${output_dir}"

    cargo run --quiet -p nekoland-msg -- completion bash > "${output_dir}/nekoland-msg.bash"
    cargo run --quiet -p nekoland-msg -- completion zsh > "${output_dir}/_nekoland-msg"
    cargo run --quiet -p nekoland-msg -- completion fish > "${output_dir}/nekoland-msg.fish"
}

case "${MODE}" in
    generate)
        echo "Generating nekoland-msg shell completions into ${COMPLETION_DIR}..."
        generate_into "${COMPLETION_DIR}"
        echo "Generated:"
        echo "  ${COMPLETION_DIR}/nekoland-msg.bash"
        echo "  ${COMPLETION_DIR}/_nekoland-msg"
        echo "  ${COMPLETION_DIR}/nekoland-msg.fish"
        ;;
    --check)
        TEMP_DIR="$(mktemp -d)"
        trap 'rm -rf "${TEMP_DIR}"' EXIT

        echo "Checking committed nekoland-msg shell completions..."
        generate_into "${TEMP_DIR}"

        diff -u "${COMPLETION_DIR}/nekoland-msg.bash" "${TEMP_DIR}/nekoland-msg.bash"
        diff -u "${COMPLETION_DIR}/_nekoland-msg" "${TEMP_DIR}/_nekoland-msg"
        diff -u "${COMPLETION_DIR}/nekoland-msg.fish" "${TEMP_DIR}/nekoland-msg.fish"

        echo "Shell completions are up to date."
        ;;
    *)
        echo "usage: $0 [generate|--check]" >&2
        exit 1
        ;;
esac
