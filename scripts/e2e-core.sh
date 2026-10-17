#!/usr/bin/env bash
#
# Headless core e2e on a configured storage backend (#7522 acceptance).
#
# Runs the headless Rust e2e suites against the mock backend with
# OPENHUMAN_STORAGE_URL set, so the core under test keeps everything in the
# chosen backend instead of the per-workspace default:
#
#   scripts/e2e-core.sh sqlite     # sqlite:<scratch dir>
#   scripts/e2e-core.sh mongodb    # needs TSD_MONGO_URL (a local replica set)
#
# MongoDB: start one with tinystoragedrivers' .github/scripts/start-mongo-replset.sh
# (https://github.com/tinyhumansai/tinystoragedrivers, tag v0.11.0) as
#   curl -fsSL -o start-mongo-replset.sh \
#     https://raw.githubusercontent.com/tinyhumansai/tinystoragedrivers/9eeef54aea403ac07461a612189bdf4982782823/.github/scripts/start-mongo-replset.sh
#   bash start-mongo-replset.sh 27017
# and export TSD_MONGO_URL=mongodb://127.0.0.1:27017/tsd_test?directConnection=true
# (the storage-mongodb workflow does exactly this). The URL must be loopback:
# the run creates a database of its own and writes to it.
#
# What runs: `tests/tinymcp_storage_smoke_e2e.rs` spawns the real `openhuman-core
# serve` twice on one workspace with OPENHUMAN_STORAGE_URL set, loads the
# registry-pinned TinyMCP cdylib (staged and digest-verified, no network once
# staged), adds a server, restarts and lists it. Extra arguments go to the test
# binary.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

extra_features=""
driver="${1:-sqlite}"
[ $# -gt 0 ] && shift

scratch="$(mktemp -d "$REPO_ROOT/target/e2e-core.XXXXXX" 2>/dev/null || { mkdir -p "$REPO_ROOT/target" && mktemp -d "$REPO_ROOT/target/e2e-core.XXXXXX"; })"
trap 'rm -rf "$scratch"' EXIT

case "$driver" in
  sqlite)
    export OPENHUMAN_STORAGE_URL="sqlite:$scratch/storage"
    ;;
  mongodb)
    base="${TSD_MONGO_URL:-}"
    if [ -z "$base" ]; then
      echo "[e2e-core] ERROR: TSD_MONGO_URL is not set (a loopback replica set is required)." >&2
      exit 2
    fi
    case "$base" in
      mongodb://*) ;;
      *) echo "[e2e-core] ERROR: TSD_MONGO_URL must be a mongodb:// URL." >&2; exit 2 ;;
    esac
    # Judge every seed host in full (userinfo, port, query stripped), not a
    # prefix: mongodb://localhost.example.com or a second remote seed in a
    # comma-separated list must not pass as loopback.
    hostlist="${base#mongodb://}"
    hostlist="${hostlist%%/*}"
    hostlist="${hostlist%%\?*}"
    hostlist="${hostlist##*@}"
    old_ifs="$IFS"
    IFS=','
    for hostpart in $hostlist; do
      IFS="$old_ifs"
      case "$hostpart" in
        \[::1\]|\[::1\]:*) ;;
        *) hostpart="${hostpart%%:*}" ;;
      esac
      case "$hostpart" in
        localhost|127.0.0.1|\[::1\]|\[::1\]:*) ;;
        *) echo "[e2e-core] ERROR: TSD_MONGO_URL must name only local throwaway servers." >&2; exit 2 ;;
      esac
      IFS=','
    done
    IFS="$old_ifs"
    db="oh_e2e_core_$(date +%s)_$$"
    head="${base%%\?*}"
    query=""
    [ "$head" != "$base" ] && query="?${base#*\?}"
    authority="${head#mongodb://}"
    authority="${authority%%/*}"
    export OPENHUMAN_STORAGE_URL="mongodb://${authority}/${db}${query}"
    # The product build carries the sqlite driver only; add mongodb.
    extra_features=",storage-mongodb"
    ;;
  *)
    echo "usage: $0 [sqlite|mongodb] [test-binary args]" >&2
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
features="$(bash "$SCRIPT_DIR/ci/product-features.sh")$extra_features"
bash "$SCRIPT_DIR/ci-cancel-aware.sh" cargo test -p openhuman-cli \
  --features "$features,e2e-test-support" --test tinymcp_storage_smoke_e2e -- \
  --test-threads=1 "$@"
