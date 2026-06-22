#!/usr/bin/env bash
#
# Rust E2E suite — the cargo-test counterpart to the Tauri E2E specs.
#
# Boots the mock backend (same `scripts/mock-api-server.mjs` the Tauri
# E2E uses) on a fixed port and then runs each `tests/*_e2e.rs`
# integration test against it. Tests that don't currently consume the
# mock backend still run here so we keep one place to add new
# mock-driven integration tests over time.
#
# This is invoked from:
#   - `pnpm test:rust:e2e` (local dev + Docker)
#   - `.github/workflows/e2e.yml` (the `rust-e2e-linux` job)
#
# Usage:
#   ./scripts/test-rust-e2e.sh                       # all default e2e tests
#   ./scripts/test-rust-e2e.sh --suite json_rpc_e2e  # one specific suite
#   ./scripts/test-rust-e2e.sh -- --ignored          # extra cargo-test args
#
# Env knobs:
#   MOCK_API_PORT  — mock backend port (default 18505).
#   MOCK_LOG       — path for mock server stdout/stderr.
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# The full set of `tests/*_e2e.rs` files. The default runner executes them
# serially so CI does not link several large integration binaries at once.
# Tests guarded by `#[ignore]` stay skipped unless the caller passes
# `-- --ignored`.
ALL_E2E_SUITES=(
  agent_retrieval_e2e
  autocomplete_memory_e2e
  calendar_grounding_e2e
  config_auth_app_state_connectivity_e2e
  composio_post_oauth_retry_e2e
  cwd_jail_e2e
  domain_modules_e2e
  embeddings_rpc_e2e
  inference_provider_e2e
  json_rpc_e2e
  keyring_secretstore_fresh_e2e
  keyring_secretstore_e2e
  linux_cef_deb_runtime_e2e
  live_routing_e2e
  mcp_registry_e2e
  mcp_setup_e2e
  memory_artifacts_e2e
  memory_graph_sync_e2e
  memory_roundtrip_e2e
  memory_sources_e2e
  memory_tree_summarizer_e2e
  memory_tree_walk_e2e
  ollama_embeddings_fallback_e2e
  screen_intelligence_vision_e2e
  skill_registry_e2e
  subconscious_e2e
  worker_b_domain_e2e
  worker_c_modules_e2e
)

# Parse args: --suite <name> can be passed multiple times to filter.
# Everything after `--` is forwarded to cargo test as test-binary args.
SUITES=()
EXTRA_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --suite)
      # Guard against `set -u` blowing up on `--suite` with no argument
      # — turning that into a clear usage error is friendlier than
      # the cryptic "$2: unbound variable" from bash.
      if [ $# -lt 2 ] || [ -z "${2:-}" ]; then
        echo "[rust-e2e] ERROR: --suite requires a test name (e.g. --suite json_rpc_e2e)" >&2
        exit 2
      fi
      SUITES+=("$2")
      shift 2
      ;;
    --)
      shift
      EXTRA_ARGS+=("$@")
      break
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done
if [ "${#SUITES[@]}" -eq 0 ]; then
  SUITES=("${ALL_E2E_SUITES[@]}")
fi

MOCK_API_PORT="${MOCK_API_PORT:-18505}"
MOCK_API_URL="http://127.0.0.1:${MOCK_API_PORT}"
MOCK_LOG="${MOCK_LOG:-/tmp/openhuman-rust-e2e-mock.log}"
MOCK_PID=""

cleanup() {
  if [ -n "$MOCK_PID" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "[rust-e2e] Starting mock API server on ${MOCK_API_URL} ..."
node "$SCRIPT_DIR/mock-api-server.mjs" --port "$MOCK_API_PORT" >"$MOCK_LOG" 2>&1 &
MOCK_PID=$!

for i in $(seq 1 30); do
  if curl -sf "${MOCK_API_URL}/__admin/health" >/dev/null 2>&1; then
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "[rust-e2e] ERROR: mock API server did not become healthy in time." >&2
    echo "[rust-e2e] See logs: $MOCK_LOG" >&2
    exit 1
  fi
  sleep 1
done
echo "[rust-e2e] Mock backend healthy."

export BACKEND_URL="$MOCK_API_URL"
export VITE_BACKEND_URL="$MOCK_API_URL"

cd "$REPO_ROOT"
source "$HOME/.cargo/env" 2>/dev/null || true
RUSTC_BIN="$(command -v rustc)"
CARGO_BIN="${CARGO_BIN:-$(dirname "$RUSTC_BIN")/cargo}"
if [ ! -x "$CARGO_BIN" ]; then
  CARGO_BIN="$(command -v cargo)"
fi

echo "[rust-e2e] Running ${#SUITES[@]} suite(s) serially."
for suite in "${SUITES[@]}"; do
  if [ "${#EXTRA_ARGS[@]}" -gt 0 ]; then
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite -- ${EXTRA_ARGS[*]}"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test --manifest-path Cargo.toml --test "$suite" -- "${EXTRA_ARGS[@]}"
  else
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test --manifest-path Cargo.toml --test "$suite"
  fi
done
