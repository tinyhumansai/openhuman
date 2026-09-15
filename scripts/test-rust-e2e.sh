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
  agent_approval_memory_coverage_e2e
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
  memory_roundtrip_e2e
  memory_sources_e2e
  observability_wallet_expected_e2e
  skill_registry_e2e
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

# The agent-harness E2E surface drives very large async futures in debug builds
# (the typed sub-agent runner + the full agentic brain turn exercised by
# json_rpc_meet_agent_session_lifecycle). The default Rust test-thread stack
# (2 MiB) overflows on that dispatch depth — a stack overflow in otherwise-correct
# tests, not a logic failure. Mirror scripts/test-rust-with-mock.sh and give the
# suite a larger stack unless the caller already pinned one explicitly.
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

cd "$REPO_ROOT"
source "$HOME/.cargo/env" 2>/dev/null || true
RUSTC_BIN="$(command -v rustc)"
CARGO_BIN="${CARGO_BIN:-$(dirname "$RUSTC_BIN")/cargo}"
if [ ! -x "$CARGO_BIN" ]; then
  CARGO_BIN="$(command -v cargo)"
fi

# This is the product E2E runner, not the slim contributor build. Several
# suites below intentionally require product gates (for example `voice` for
# `json_rpc_e2e` and `memory-git` for memory artifact coverage), so invoking
# them with Cargo's default feature set makes the runner fail before a test can
# execute. Keep this list in the same canonical source as `pnpm test:rust`.
PRODUCT_FEATURES="$(bash "$REPO_ROOT/scripts/ci/product-features.sh")"

# Memory-backed RPC tests use the pinned native module. Build the submodule
# artifact when CI/local callers have not supplied an explicit release pin;
# otherwise the clean E2E container falls back to unavailable release metadata.
if [ -z "${TINYMEMORY_TEST_MODULE:-}" ]; then
  memory_manifest="vendor/tinymemory/crates/tinymemory-module/Cargo.toml"
  memory_module="vendor/tinymemory/crates/tinymemory-module/target/release/libtinymemory_module.so"
  echo "[rust-e2e] Building pinned TinyMemory test module ..."
  "$CARGO_BIN" build --release --manifest-path "$memory_manifest"
  export TINYMEMORY_TEST_MODULE="$REPO_ROOT/$memory_module"
fi

echo "[rust-e2e] Running ${#SUITES[@]} suite(s) serially."

run_json_rpc_e2e_suite() {
  # JSON-RPC scenarios mutate process-global provider routes and runtime
  # configuration. Run every case in a fresh test process so a provider set by
  # one scenario cannot affect the routing assertions in another.
  #
  # Enumerate first, as its own checked step. The names used to be read straight
  # from a process substitution, whose exit status nothing sees: a target that
  # failed to compile listed nothing, the loop ran zero times, and the suite
  # reported success. A listing that fails part-way is refused whole, so a
  # partial list is never run as though it were complete.
  local listing test_name
  if ! listing="$(
    "$CARGO_BIN" test --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test json_rpc_e2e -- --list
  )"; then
    echo "[rust-e2e] ERROR: could not enumerate json_rpc_e2e tests; refusing to run an empty or partial list." >&2
    return 1
  fi

  while IFS= read -r test_name; do
    [ -n "$test_name" ] || continue
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test json_rpc_e2e $test_name"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test json_rpc_e2e "$test_name" -- \
      --exact --test-threads=1 "${EXTRA_ARGS[@]}"
  done <<<"$(printf '%s\n' "$listing" | sed -n 's/: test$//p')"
}

for suite in "${SUITES[@]}"; do
  if [ "$suite" = "json_rpc_e2e" ]; then
    run_json_rpc_e2e_suite
    continue
  fi

  if [ "${#EXTRA_ARGS[@]}" -gt 0 ]; then
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite -- ${EXTRA_ARGS[*]}"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test "$suite" -- "${EXTRA_ARGS[@]}"
  else
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" --test "$suite"
  fi
done
