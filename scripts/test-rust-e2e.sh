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
# The suite list is DERIVED from `tests/*_e2e.rs` plus the `in_process_all`
# aggregate (the former in-process router suites now live under
# `tests/in_process/`), not hand-maintained.
#
# It used to be a literal list of 20 names while 29 `tests/*_e2e.rs` targets
# existed, so nine were silently absent from this runner — including
# `transcript_search_e2e`, which a coverage matrix cited as existing coverage.
# CI never had the gap: `test-reusable.yml`, `scripts/ci/rust-coverage.sh` and
# `scripts/test-rust-with-mock.sh` all discover targets with the same `find`
# used below. Only the documented LOCAL command was short, which made a local
# green weaker than it read.
#
# Polarity matters here. An include list fails UNSAFE: a new target is omitted
# until someone remembers to add it. The exclude list below fails SAFE: a new
# target runs by default and must be deliberately opted out, with a cause. Keep
# it that way, and keep it empty if you can — a target that needs an external
# service should gate itself with `#[ignore]` or an env check, which costs
# nothing here because the target still runs and simply reports zero tests.
E2E_SUITE_EXCLUDE=()

_discover_e2e_suites() {
  local name
  while IFS= read -r name; do
    local skip=0
    local excluded
    for excluded in ${E2E_SUITE_EXCLUDE[@]+"${E2E_SUITE_EXCLUDE[@]}"}; do
      [ "$name" = "$excluded" ] && skip=1 && break
    done
    [ $skip -eq 0 ] && printf '%s\n' "$name"
  done < <(
    find "$REPO_ROOT/tests" -maxdepth 1 -type f \( -name '*_e2e.rs' -o -name 'in_process_all.rs' \) -print |
      sed -e 's#.*/##' -e 's#\.rs$##' |
      sort
  )
}

ALL_E2E_SUITES=()
while IFS= read -r _suite; do
  ALL_E2E_SUITES+=("$_suite")
done < <(_discover_e2e_suites)

# Refuse to run on an empty discovery rather than reporting a vacuous success.
# `SUITES` falling back to an empty `ALL_E2E_SUITES` would run nothing and exit
# 0, which reads exactly like a passing suite.
if [ "${#ALL_E2E_SUITES[@]}" -eq 0 ]; then
  echo "[rust-e2e] ERROR: discovered 0 e2e suites under $REPO_ROOT/tests." >&2
  echo "           Expected tests/*_e2e.rs targets; refusing to report success." >&2
  exit 2
fi

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

# Module-backed Composio coverage must use the pinned local artifact as well.
# Without this override the core resolves TinyConnectors through release
# metadata, turning an otherwise hermetic mock-backend suite into a network
# dependency and permanently faulting that process when the lookup fails.
if [ -z "${TINYCONNECTORS_TEST_MODULE:-}" ]; then
  connectors_manifest="vendor/tinyconnectors/crates/tinyconnectors/Cargo.toml"
  connectors_module="vendor/tinyconnectors/target/release/libtinyconnectors.so"
  echo "[rust-e2e] Building pinned TinyConnectors test module ..."
  "$CARGO_BIN" build --release --manifest-path "$connectors_manifest"
  export TINYCONNECTORS_TEST_MODULE="$REPO_ROOT/$connectors_module"
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
  local listing test_names test_name
  if ! listing="$(
    "$CARGO_BIN" test --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test json_rpc_e2e -- --list
  )"; then
    echo "[rust-e2e] ERROR: could not enumerate json_rpc_e2e tests; refusing to run an empty or partial list." >&2
    return 1
  fi

  test_names="$(printf '%s\n' "$listing" | sed -n 's/: test$//p')"
  if [ -z "$test_names" ]; then
    echo "[rust-e2e] ERROR: json_rpc_e2e enumeration returned no tests; refusing to report success." >&2
    return 1
  fi

  while IFS= read -r test_name; do
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test json_rpc_e2e $test_name"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test json_rpc_e2e "$test_name" -- \
      --exact --test-threads=1 "${EXTRA_ARGS[@]}"
  done <<<"$test_names"
}

run_in_process_modules() {
  # `in_process_all` folds ~20 former `tests/*.rs` targets into one binary. Run
  # each module in its own process, as those targets did, so process globals
  # (the transport install, the RPC bearer, module singletons) stay per suite.
  local module
  while IFS= read -r module; do
    [ -n "$module" ] || continue
    echo "[rust-e2e]   in_process module: ${module}"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test in_process_all -- "${module}::" ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
  done < <(
    find "$REPO_ROOT/tests/in_process" -maxdepth 1 -type f -name '*.rs' -print |
      sed -e 's#.*/##' -e 's#\.rs$##' |
      sort
  )
}

for suite in "${SUITES[@]}"; do
  if [ "$suite" = "json_rpc_e2e" ]; then
    run_json_rpc_e2e_suite
    continue
  fi
  if [ "$suite" = "in_process_all" ]; then
    run_in_process_modules
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
