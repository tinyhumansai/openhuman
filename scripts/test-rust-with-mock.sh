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
# trap `--features bin-tools` already guards for the `crates/openhuman-cli/src/bin/` targets.
# Source of truth: scripts/ci/product-features.txt.
PRODUCT_FEATURES="$(bash "$REPO_ROOT/scripts/ci/product-features.sh")"

# Local module artifacts use the host's native library extension. The wallet
# fixture is a published, checksum-pinned archive for that same host.
case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    module_ext=dylib
    if [ "$(sw_vers -productVersion | cut -d. -f1)" -ge 26 ]; then
      wallet_platform=macos-26-arm64
      wallet_sha256=6409e1bfcd542f4fba5d867bd6b06b094a24d8e9bd994aae46894dcd979d3291
    else
      wallet_platform=macos-15-arm64
      wallet_sha256=06d66492a7ba9b3793810ff9e644231deacf6fbdf9e32008fcef866f8228cbd2
    fi
    ;;
  Darwin:x86_64)
    module_ext=dylib
    if [ "$(sw_vers -productVersion | cut -d. -f1)" -ge 26 ]; then
      wallet_platform=macos-26-x86_64
      wallet_sha256=3ad698d3f6ee549ee4c724717e78a2b16b5504ea4bea89af3ccf06cf3e7c88dc
    else
      wallet_platform=macos-15-x86_64
      wallet_sha256=db4e6b2dae244059ef9bf3401faadb6d58bd6f70bb6279135944a41bb5e3b4f5
    fi
    ;;
  Linux:x86_64)
    module_ext=so
    wallet_platform=ubuntu-22.04-x86_64
    wallet_sha256=4410a0d88d49f3553c01ef6f0ced3a2997388d698d57d08bc129b82807e57016
    ;;
  Linux:aarch64|Linux:arm64)
    module_ext=so
    wallet_platform=ubuntu-22.04-arm64
    wallet_sha256=90e391b78ebb25650d7d9cff6440869e045298d1b4976e7d61b6b20b9ca8a3d4
    ;;
  *)
    echo "Unsupported native-module test host: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

# Tokenjuice JSON-RPC coverage loads the production native module. Keep the
# test run hermetic by building the pinned submodule instead of falling back
# to GitHub release metadata.
if [ -z "${TINYJUICE_TEST_MODULE:-}" ]; then
  juice_manifest="vendor/tinyjuice/crates/tinyjuice-module/Cargo.toml"
  juice_module="vendor/tinyjuice/target/release/libtinyjuice_module.$module_ext"
  echo "Building TinyJuice test module from the pinned submodule ..."
  cargo build --release --manifest-path "$juice_manifest"
  export TINYJUICE_TEST_MODULE="$REPO_ROOT/$juice_module"
fi

# Search JSON-RPC/agent coverage calls the TinySearch module against the mock
# backend's managed Exa/Gemini routes. Build the pinned submodule so the suite
# never depends on a published release.
if [ -z "${TINYSEARCH_TEST_MODULE:-}" ]; then
  search_manifest="vendor/tinysearch/crates/tinysearch/Cargo.toml"
  search_module="vendor/tinysearch/target/release/libtinysearch.$module_ext"
  echo "Building TinySearch test module from the pinned submodule ..."
  cargo build --release --manifest-path "$search_manifest"
  export TINYSEARCH_TEST_MODULE="$REPO_ROOT/$search_module"
fi

# Wallet JSON-RPC E2E sends a recovery phrase only to an attested module. Build
# artifacts are deliberately not treated as release-pinned recipients, so use
# the checksum-pinned release archive and its accompanying `modules.toml`.
# Keyed by version: the presence check below would otherwise keep serving a
# library from an earlier release after the pin moves, and the wallet only
# attests the pinned release's library.
wallet_version="0.5.3"
wallet_dir="$REPO_ROOT/target/test-modules/tinywallet/$wallet_version"
wallet_archive="$wallet_dir/tinywallet-module-$wallet_version-$wallet_platform.tar.gz"
if [ ! -f "$wallet_dir/libtinywallet_module.$module_ext" ]; then
  echo "Downloading the pinned TinyWallet test module ..."
  mkdir -p "$wallet_dir"
  curl --fail --location --silent --show-error \
    "https://github.com/tinyhumansai/tinywallet/releases/download/v${wallet_version}/$(basename "$wallet_archive")" \
    --output "$wallet_archive"
  # macOS ships a `sha256sum` that does not accept GNU's stdin check mode.
  if command -v shasum >/dev/null 2>&1; then
    echo "${wallet_sha256}  $wallet_archive" | shasum -a 256 -c
  else
    echo "${wallet_sha256}  $wallet_archive" | sha256sum --check
  fi
  tar -xzf "$wallet_archive" -C "$wallet_dir"
fi
export OPENHUMAN_MODULE_PATH="$wallet_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

if [ -z "${TINYCONNECTORS_TEST_MODULE:-}" ]; then
  connectors_manifest="vendor/tinyconnectors/crates/tinyconnectors/Cargo.toml"
  connectors_module="$REPO_ROOT/vendor/tinyconnectors/target/release/libtinyconnectors.$module_ext"
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
         [ "$module" = "composio_tools_direct_raw_coverage_e2e" ] ||
         [ "$module" = "tools_composio_large_round25_raw_coverage_e2e" ]; } &&
       [ -z "${TINYCONNECTORS_TEST_MODULE:-}" ]; then
      TINYCONNECTORS_TEST_MODULE="$connectors_module" \
        cargo_test --test raw_coverage_all -- "${module}::" --test-threads=1 "$@"
    else
      cargo_test --test raw_coverage_all -- "${module}::" --test-threads=1 "$@"
    fi
  done < <(raw_coverage_modules)
}

in_process_modules() {
  find tests/in_process -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/in_process/##' -e 's#\.rs$##' |
    sort
}

run_in_process_modules() {
  # ~20 former `tests/*.rs` targets now share the `in_process_all` binary. Run
  # each module in its own cargo process so the process-global RPC bearer,
  # backend transport and module singletons stay per suite, as they were.
  while IFS= read -r module; do
    [ -n "$module" ] || continue
    echo "[test-rust-with-mock] in-process module: ${module}"
    TINYCONNECTORS_TEST_MODULE="${TINYCONNECTORS_TEST_MODULE:-$connectors_module}" \
      cargo_test --test in_process_all -- "${module}::" "$@"
  done < <(in_process_modules)
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

run_build_only_reaper_test() {
  # Building the runtime installs process-global context that cannot be reset.
  # Keep this real build-path regression in a fresh process so it cannot narrow
  # the DomainSet observed by later registry and domain tests.
  cargo_test --lib \
    "openhuman::agent::tinyagents::reaper::tests::a_build_only_runtime_is_swept_before_it_can_be_invoked" \
    -- --exact --test-threads=1 "$@"
}

run_full_suite() {
  # Several unit fixtures mutate process-wide state (provider overrides and
  # temporary executable paths). Keep this aggregate invocation deterministic;
  # integration targets below retain their own, narrower isolation strategies.
  TINYCONNECTORS_TEST_MODULE="${TINYCONNECTORS_TEST_MODULE:-$connectors_module}" \
    cargo_test --lib -- --test-threads=1 \
    --skip a_build_only_runtime_is_swept_before_it_can_be_invoked "$@"
  # The core binary and the developer bins live in `crates/openhuman-cli`;
  # `--workspace --bins` picks them up there (plus the TUI binary).
  cargo_test --bins -- --test-threads=1 "$@"
  run_build_only_reaper_test "$@"
  cargo_test --doc -- "$@"

  while IFS= read -r target; do
    [ -n "$target" ] || continue
    if [ "$target" = "raw_coverage_all" ]; then
      # These suites used to run as separate integration-test binaries. Run
      # each generated module filter in its own cargo process so local
      # `pnpm test:rust` preserves the same process-global isolation as CI.
      run_raw_coverage_modules "$@"
    elif [ "$target" = "in_process_all" ]; then
      run_in_process_modules "$@"
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
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "in_process_all" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  run_in_process_modules "$@"
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "json_rpc_e2e" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  run_json_rpc_e2e "$@"
else
  cargo_test "$@"
fi
