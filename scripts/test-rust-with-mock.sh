#!/usr/bin/env bash
#
# Run Rust tests against the shared mock backend.
#
# Usage:
#   ./scripts/test-rust-with-mock.sh
#   ./scripts/test-rust-with-mock.sh --test json_rpc_e2e
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

MOCK_API_PORT="${MOCK_API_PORT:-18505}"
MOCK_API_URL="http://127.0.0.1:${MOCK_API_PORT}"
MOCK_LOG="${MOCK_LOG:-/tmp/openhuman-mock-api.log}"
MOCK_PID=""

cleanup() {
  if [ -n "$MOCK_PID" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "Starting mock API server on ${MOCK_API_URL} ..."
node "$SCRIPT_DIR/mock-api-server.mjs" --port "$MOCK_API_PORT" >"$MOCK_LOG" 2>&1 &
MOCK_PID=$!

for i in $(seq 1 30); do
  if curl -sf "${MOCK_API_URL}/__admin/health" >/dev/null 2>&1; then
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "ERROR: mock API server did not become healthy in time." >&2
    echo "See logs: $MOCK_LOG" >&2
    exit 1
  fi
  sleep 1
done

export BACKEND_URL="$MOCK_API_URL"
export VITE_BACKEND_URL="$MOCK_API_URL"
# The agent harness test surface includes very large async futures in debug
# builds (notably the typed sub-agent runner). The default Rust test-thread
# stack can be too small on Apple Silicon debug runs, leading to a stack
# overflow in otherwise-correct tests. Give the full suite a larger stack
# unless the caller already pinned one explicitly.
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

# The tinyagents harness is the agent engine on every build now (issue #4249);
# the suite exercises it by default. Set OPENHUMAN_AGENT_GRAPH_{TINYAGENTS,CHANNEL,
# SUBAGENT}=0 to force the (being-removed) legacy engine during the transition.

echo "Running Rust tests with BACKEND_URL=$BACKEND_URL and RUST_MIN_STACK=$RUST_MIN_STACK"
cd "$REPO_ROOT"
# Only source rustup's env if it actually exists. With `set -e`, sourcing a
# *missing* file is a fatal error in a non-interactive shell and the trailing
# `|| true` does NOT catch it — the shell exits before the `||` is evaluated.
# On machines where Rust came from Homebrew/system packages (no rustup) there is
# no ~/.cargo/env, so the old unconditional `source` silently aborted the script
# *before* `cargo test` ever ran — and looked like a green "OK" while no tests
# actually executed.
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

cargo_test() {
  cargo test --manifest-path Cargo.toml --workspace "$@"
}

integration_test_targets() {
  find tests -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/##' -e 's#\.rs$##' |
    sort
}

raw_coverage_modules() {
  find tests/raw_coverage -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/raw_coverage/##' -e 's#\.rs$##' |
    sort
}

run_raw_coverage_modules() {
  while IFS= read -r module; do
    [ -n "$module" ] || continue
    echo "[test-rust-with-mock] raw coverage module: ${module}"
    cargo_test --test raw_coverage_all -- "${module}::" --test-threads=1 "$@"
  done < <(raw_coverage_modules)
}

run_full_suite() {
  cargo_test --lib --bins -- "$@"
  cargo_test --doc -- "$@"

  while IFS= read -r target; do
    [ -n "$target" ] || continue
    if [ "$target" = "raw_coverage_all" ]; then
      # These suites used to run as separate integration-test binaries. Run
      # each generated module filter in its own cargo process so local
      # `pnpm test:rust` preserves the same process-global isolation as CI.
      run_raw_coverage_modules "$@"
    else
      cargo_test --test "$target" -- "$@"
    fi
  done < <(integration_test_targets)
}

if [ "$#" -eq 0 ]; then
  run_full_suite
elif [ "$1" = "--" ]; then
  shift
  run_full_suite "$@"
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "raw_coverage_all" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  run_raw_coverage_modules "$@"
else
  cargo_test "$@"
fi
