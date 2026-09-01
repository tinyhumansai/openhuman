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

# The TinyAgents harness is the only agent engine on every build (issue #4249),
# so the suite exercises the production path without a legacy escape hatch.

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

# `pnpm test:rust` is the "does the product still work" runner, so it selects
# the product's gates rather than `[features] default`, which is the smaller
# contributor set. Without them the four `required-features` integration
# targets (json_rpc_e2e, raw_coverage_all, observability_smoke,
# x402_twit_sh_live) are silently SKIPPED and the run still exits 0 — the same
# trap `--features bin-tools` already guards for the `src/bin/` targets.
# Source of truth: scripts/ci/product-features.txt.
PRODUCT_FEATURES="$(bash "$REPO_ROOT/scripts/ci/product-features.sh")"

# The product test surface exercises memory through its native module. CI builds
# the pinned submodule and supplies this explicit override; mirror that setup
# locally so the full runner never falls back to GitHub release metadata (which
# makes an otherwise hermetic mock-backend suite network-bound).
if [ -z "${TINYMEMORY_TEST_MODULE:-}" ]; then
  memory_manifest="vendor/tinymemory/crates/tinymemory-module/Cargo.toml"
  memory_module="vendor/tinymemory/crates/tinymemory-module/target/release/libtinymemory_module.so"
  echo "Building TinyMemory test module from the pinned submodule ..."
  cargo build --release --manifest-path "$memory_manifest"
  export TINYMEMORY_TEST_MODULE="$REPO_ROOT/$memory_module"
fi

# Tokenjuice JSON-RPC coverage loads the production native module. Keep the
# test run hermetic by building the pinned submodule instead of falling back
# to GitHub release metadata.
if [ -z "${TINYJUICE_TEST_MODULE:-}" ]; then
  juice_manifest="vendor/tinyjuice/crates/tinyjuice-module/Cargo.toml"
  juice_module="vendor/tinyjuice/target/release/libtinyjuice_module.so"
  echo "Building TinyJuice test module from the pinned submodule ..."
  cargo build --release --manifest-path "$juice_manifest"
  export TINYJUICE_TEST_MODULE="$REPO_ROOT/$juice_module"
fi

# Wallet JSON-RPC E2E sends a recovery phrase only to an attested module. Build
# artifacts are deliberately not treated as release-pinned recipients, so use
# the checksum-pinned release archive and its accompanying `modules.toml`.
wallet_dir="$REPO_ROOT/target/test-modules/tinywallet"
wallet_archive="$wallet_dir/tinywallet-module-0.5.1-ubuntu-22.04-x86_64.tar.gz"
wallet_sha256="88b63685cab8a622416f24f1ad569153f249d6d74732ff33c79e4021cf64a611"
if [ ! -f "$wallet_dir/libtinywallet_module.so" ]; then
  echo "Downloading the pinned TinyWallet test module ..."
  mkdir -p "$wallet_dir"
  curl --fail --location --silent --show-error \
    "https://github.com/tinyhumansai/tinywallet/releases/download/v0.5.1/$(basename "$wallet_archive")" \
    --output "$wallet_archive"
  echo "${wallet_sha256}  $wallet_archive" | sha256sum --check
  tar -xzf "$wallet_archive" -C "$wallet_dir"
fi
export OPENHUMAN_MODULE_PATH="$wallet_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

if [ -z "${TINYCONNECTORS_TEST_MODULE:-}" ]; then
  connectors_manifest="vendor/tinyconnectors/crates/tinyconnectors/Cargo.toml"
  connectors_module="$REPO_ROOT/vendor/tinyconnectors/target/release/libtinyconnectors.so"
  echo "Building TinyConnectors test module from the pinned submodule ..."
  cargo build --release --manifest-path "$connectors_manifest"
fi

cargo_test() {
  cargo test --manifest-path Cargo.toml --workspace \
    --features "${PRODUCT_FEATURES},bin-tools" "$@"
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
    # Most Composio raw-coverage modules explicitly exercise the absent-module
    # path. These groups verify the host-to-module round trip, so inject the
    # pinned connector only for their processes.
    if { [ "$module" = "composio_credentials_state_raw_coverage_e2e" ] ||
         [ "$module" = "composio_ops_raw_coverage_e2e" ] ||
         [ "$module" = "tools_composio_large_round25_raw_coverage_e2e" ]; } &&
       [ -z "${TINYCONNECTORS_TEST_MODULE:-}" ]; then
      TINYCONNECTORS_TEST_MODULE="$connectors_module" \
        cargo_test --test raw_coverage_all -- "${module}::" --test-threads=1 "$@"
    else
      cargo_test --test raw_coverage_all -- "${module}::" --test-threads=1 "$@"
    fi
  done < <(raw_coverage_modules)
}

run_json_rpc_e2e() {
  # The JSON-RPC E2E binary intentionally changes process-global environment
  # and runtime configuration. Run each case in a fresh test process so a
  # provider route persisted by one scenario cannot affect another one.
  while IFS= read -r test_name; do
    [ -n "$test_name" ] || continue
    echo "[test-rust-with-mock] JSON-RPC E2E test: ${test_name}"
    cargo_test --test json_rpc_e2e "$test_name" -- --test-threads=1 "$@"
  done < <(cargo_test --test json_rpc_e2e -- --list | sed -n 's/: test$//p')
}

run_archivist_tree_tests() {
  local test_name
  for test_name in \
    phase2_no_per_turn_tree_write \
    phase2_exactly_one_tree_ingest_per_segment_close \
    phase2_provenance_stamped_on_leaf_and_source_id_is_constant \
    phase2_ingested_content_is_raw_prose_not_recap \
    phase2_flush_also_triggers_tree_ingest; do
    echo "[test-rust-with-mock] archivist tree test: ${test_name}"
    cargo_test --lib "openhuman::agent::harness::archivist::tests::part_01_tests::${test_name}" -- --exact --test-threads=1 "$@"
  done
}

run_full_suite() {
  # Several unit fixtures mutate process-wide state (provider overrides and
  # temporary executable paths). Keep this aggregate invocation deterministic;
  # integration targets below retain their own, narrower isolation strategies.
  TINYCONNECTORS_TEST_MODULE="${TINYCONNECTORS_TEST_MODULE:-$connectors_module}" \
    cargo_test --lib --bins -- --test-threads=1 \
    --skip phase2_no_per_turn_tree_write \
    --skip phase2_exactly_one_tree_ingest_per_segment_close \
    --skip phase2_provenance_stamped_on_leaf_and_source_id_is_constant \
    --skip phase2_ingested_content_is_raw_prose_not_recap \
    --skip phase2_flush_also_triggers_tree_ingest "$@"
  run_archivist_tree_tests "$@"
  cargo_test --doc -- "$@"

  while IFS= read -r target; do
    [ -n "$target" ] || continue
    if [ "$target" = "raw_coverage_all" ]; then
      # These suites used to run as separate integration-test binaries. Run
      # each generated module filter in its own cargo process so local
      # `pnpm test:rust` preserves the same process-global isolation as CI.
      run_raw_coverage_modules "$@"
    elif [ "$target" = "json_rpc_e2e" ]; then
      run_json_rpc_e2e "$@"
    else
      TINYCONNECTORS_TEST_MODULE="${TINYCONNECTORS_TEST_MODULE:-$connectors_module}" \
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
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "json_rpc_e2e" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  run_json_rpc_e2e "$@"
else
  cargo_test "$@"
fi
