#!/usr/bin/env bash
#
# Headless core e2e on a configured storage backend (#7522 acceptance).
#
# Runs the headless Rust e2e suites against the mock backend with
# OPENHUMAN_STORAGE_URL set, so the core under test keeps everything in the
# chosen backend instead of the per-workspace default:
#
#   scripts/e2e-core.sh [sqlite]   # sqlite:<scratch dir>
#
# SQLite is the only driver the host runs end to end. Per-driver conformance
# (including MongoDB against a replica set) runs upstream in
# tinystoragedrivers' CI.
#
# What runs: `tests/tinymcp_storage_smoke_e2e.rs` spawns the real `openhuman-core
# serve` twice on one workspace with OPENHUMAN_STORAGE_URL set, loads the
# registry-pinned TinyMCP cdylib (staged and digest-verified, no network once
# staged), adds a server, restarts and lists it. Extra arguments go to the test
# binary.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

driver="${1:-sqlite}"
[ $# -gt 0 ] && shift

scratch="$(mktemp -d "$REPO_ROOT/target/e2e-core.XXXXXX" 2>/dev/null || { mkdir -p "$REPO_ROOT/target" && mktemp -d "$REPO_ROOT/target/e2e-core.XXXXXX"; })"
trap 'rm -rf "$scratch"' EXIT

case "$driver" in
  sqlite)
    export OPENHUMAN_STORAGE_URL="sqlite:$scratch/storage"
    ;;
  *)
    echo "usage: $0 [sqlite] [test-binary args]" >&2
    exit 2
    ;;
esac

# Shared backends keep secrets under the keyring master key and fail closed
# without one; CI has no OS keychain.
export OPENHUMAN_KEYRING_MASTER_KEY="${OPENHUMAN_KEYRING_MASTER_KEY:-$(printf '55%.0s' $(seq 1 32))}"

echo "[e2e-core] driver=$driver (OPENHUMAN_STORAGE_URL set)"

module_target="$(bash "$SCRIPT_DIR/ci/native-test-module-platform.sh")"
IFS=$'\t' read -r module_ext module_platform <<< "$module_target"
export REPO_ROOT
# shellcheck source=modules/stage-local-modules.sh
source "$SCRIPT_DIR/modules/stage-local-modules.sh"
tinymcp_dir="$(stage_release_module tinymcp "$module_platform" "$module_ext")"
export OPENHUMAN_MODULE_PATH="$tinymcp_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"
cd "$REPO_ROOT"
features="$(bash "$SCRIPT_DIR/ci/product-features.sh")"
bash "$SCRIPT_DIR/ci-cancel-aware.sh" cargo test -p openhuman-cli \
  --features "$features,e2e-test-support" --test tinymcp_storage_smoke_e2e -- \
  --test-threads=1 "$@"
