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

native_module_target="$(bash "$SCRIPT_DIR/ci/native-test-module-platform.sh")"
IFS=$'\t' read -r module_ext wallet_platform <<< "$native_module_target"

# shellcheck source=modules/stage-local-modules.sh
source "$SCRIPT_DIR/modules/stage-local-modules.sh"

# Connectors, tokenjuice, search, hosts and MCP: the registry-pinned release,
# as in CI (#7618: no module is compiled from source there). With
# OPENHUMAN_MODULE_SOURCE=source they are built from the pinned submodules
# instead (scripts/modules/build.mjs), which is how a module change is tried
# against the core locally.
local_source_ids=()
if local_module_source_is_source; then
  local_source_ids+=(tinyconnectors tinyjuice tinysearch tinyhosts tinymcp tinysecurity)
else
  for released_id in tinychannels tinyjuice tinysearch tinysecurity; do
    released_dir="$(stage_release_module "$released_id" "$wallet_platform" "$module_ext")"
    if [ "$released_id" = "tinychannels" ]; then
      tinychannels_dir="$released_dir"
    fi
    export OPENHUMAN_MODULE_PATH="$released_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
  done
fi

# TinyChannels host unit tests use the pinned module artifact through the
# regular loader. Its source module is not linked into the core: tinychannels-
# module currently requires TinyBus 0.1.4 while this host pins 0.1.3.
if local_module_source_is_source; then
  released_dir="$(stage_release_module tinychannels "$wallet_platform" "$module_ext")"
  tinychannels_dir="$released_dir"
  export OPENHUMAN_MODULE_PATH="$released_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
fi

# Wallet JSON-RPC E2E sends a recovery phrase only to an attested module. Build
# artifacts are deliberately not treated as release-pinned recipients, so use
# the checksum-pinned release archive and its accompanying `modules.toml`.
# Keyed by digest: the presence check below would otherwise keep serving a
# library from an earlier release after the pin moves, and the wallet only
# attests the pinned release's library.
wallet_asset="$(node scripts/ci/self-hosted/test-module-assets.mjs "$wallet_platform" tinywallet)"
IFS=$'\t' read -r _wallet_id wallet_url wallet_filename wallet_sha256 <<< "$wallet_asset"
wallet_dir="$REPO_ROOT/target/test-modules/tinywallet/$wallet_sha256"
wallet_archive="$wallet_dir/$wallet_filename"
if [ ! -f "$wallet_dir/libtinywallet_module.$module_ext" ]; then
  echo "Downloading the checksum-pinned TinyWallet test module ..."
  mkdir -p "$wallet_dir"
  curl --fail --location --silent --show-error "$wallet_url" --output "$wallet_archive"
  if command -v shasum >/dev/null 2>&1; then
    echo "${wallet_sha256}  $wallet_archive" | shasum -a 256 -c
  else
    echo "${wallet_sha256}  $wallet_archive" | sha256sum --check
  fi
  tar --no-same-owner -xzf "$wallet_archive" -C "$wallet_dir"
fi
export OPENHUMAN_MODULE_PATH="$wallet_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

docs_asset="$(node scripts/ci/self-hosted/test-module-assets.mjs "$wallet_platform" tinydocs)"
IFS=$'\t' read -r _docs_id docs_url docs_filename docs_sha256 <<< "$docs_asset"
docs_dir="$REPO_ROOT/target/test-modules/tinydocs/$docs_sha256"
docs_archive="$docs_dir/$docs_filename"
if [ ! -f "$docs_dir/libtinydocs_module.$module_ext" ]; then
  mkdir -p "$docs_dir"
  curl --fail --location --silent --show-error "$docs_url" --output "$docs_archive"
  if command -v shasum >/dev/null 2>&1; then
    echo "${docs_sha256}  $docs_archive" | shasum -a 256 -c
  else
    echo "${docs_sha256}  $docs_archive" | sha256sum --check
  fi
  tar --no-same-owner -xzf "$docs_archive" -C "$docs_dir"
fi
export OPENHUMAN_MODULE_PATH="$docs_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

# The released TinyHosts contract test runs in its own process, outside the
# generic unit suite, so its directory goes only on that process (below).
# The recipient stays pinned to the registry digest like the other fixtures.
if ! local_module_source_is_source; then
  hosts_dir="$(stage_release_module tinyhosts "$wallet_platform" "$module_ext")"
fi

# TinyBox command analysis is required by shell-backed storage and capability
# tests. Load the published, checksum-pinned module rather than relying on the
# host's installed registry cache.
tinybox_asset="$(node scripts/ci/self-hosted/test-module-assets.mjs "$wallet_platform" tinybox)"
IFS=$'\t' read -r _tinybox_id tinybox_url tinybox_filename tinybox_sha256 <<< "$tinybox_asset"
tinybox_dir="$REPO_ROOT/target/test-modules/tinybox/$tinybox_sha256"
tinybox_archive="$tinybox_dir/$tinybox_filename"
if [ ! -f "$tinybox_dir/libtinybox.$module_ext" ]; then
  echo "Downloading the checksum-pinned TinyBox test module ..."
  mkdir -p "$tinybox_dir"
  curl --fail --location --silent --show-error "$tinybox_url" --output "$tinybox_archive"
  if command -v shasum >/dev/null 2>&1; then
    echo "${tinybox_sha256}  $tinybox_archive" | shasum -a 256 -c
  else
    echo "${tinybox_sha256}  $tinybox_archive" | sha256sum --check
  fi
  tar --no-same-owner -xzf "$tinybox_archive" -C "$tinybox_dir"
fi
export OPENHUMAN_MODULE_PATH="$tinybox_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

# The MCP registry E2E targets exercise real TinyMCP lifecycle calls through
# the host's module adapter, so they load the released, checksum-pinned module
# just as the self-hosted CI installer does (source mode builds it instead).
if ! local_module_source_is_source; then
  tinymcp_dir="$(stage_release_module tinymcp "$wallet_platform" "$module_ext")"
  export OPENHUMAN_MODULE_PATH="$tinymcp_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
else
  tinymcp_dir="$REPO_ROOT/target/module-sources/tinymcp"
fi
if [ "$module_ext" = "dll" ]; then
  if local_module_source_is_source; then
    tinymcp_artifact="$tinymcp_dir/tinymcp.dll"
  else
    tinymcp_artifact="$tinymcp_dir/tinymcp_module.dll"
  fi
elif local_module_source_is_source; then
  tinymcp_artifact="$tinymcp_dir/libtinymcp.$module_ext"
else
  tinymcp_artifact="$tinymcp_dir/libtinymcp_module.$module_ext"
fi

# Build the source modules in one call (local source mode only). The connector
# directory goes only on the processes that exercise it: most Composio
# raw-coverage groups explicitly test the absent-module path.
if [ "${#local_source_ids[@]}" -gt 0 ]; then
  stage_source_modules "${local_source_ids[@]}"
fi
if local_module_source_is_source; then
  connectors_dir="$REPO_ROOT/target/module-sources/tinyconnectors"
else
  connectors_dir="$(stage_release_module tinyconnectors "$wallet_platform" "$module_ext")"
fi
for source_id in ${local_source_ids[@]+"${local_source_ids[@]}"}; do
  if [ "$source_id" != tinyconnectors ] && [ "$source_id" != tinyhosts ]; then
    export OPENHUMAN_MODULE_PATH="$REPO_ROOT/target/module-sources/$source_id${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
  fi
done
if local_module_source_is_source; then
  hosts_dir="$REPO_ROOT/target/module-sources/tinyhosts"
fi
connectors_module_path="$connectors_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"

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
       true; then
      OPENHUMAN_MODULE_PATH="$connectors_module_path" \
        cargo_test --test raw_coverage_all -- "${module}::" --test-threads=1 "$@"
    elif [ "$module" = "automation_scheduling_e2e" ]; then
      # This target executes a scheduled shell job through the verified TinyBox
      # release, which the ordinary isolated raw-coverage processes do not see.
      OPENHUMAN_MODULE_PATH="$connectors_module_path" \
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
    OPENHUMAN_MODULE_PATH="$connectors_module_path" \
      cargo_test --test in_process_all -- "${module}::" "$@"
  done < <(in_process_modules)
}

run_json_rpc_e2e() {
  # The JSON-RPC E2E binary intentionally changes process-global environment
  # and runtime configuration. Run each case in a fresh test process so a
  # provider route persisted by one scenario cannot affect another one.
  local listing test_names test_name
  if ! listing="$(cargo_test --test json_rpc_e2e -- --list)"; then
    echo "[test-rust-with-mock] ERROR: could not enumerate json_rpc_e2e tests; refusing an empty or partial list." >&2
    return 1
  fi
  test_names="$(printf '%s\n' "$listing" | sed -n 's/: test$//p')"
  if [ -z "$test_names" ]; then
    echo "[test-rust-with-mock] ERROR: json_rpc_e2e enumeration returned no tests." >&2
    return 1
  fi
  while IFS= read -r test_name; do
    echo "[test-rust-with-mock] JSON-RPC E2E test: ${test_name}"
    cargo_test --test json_rpc_e2e "$test_name" -- --exact --test-threads=1 "$@"
  done <<<"$test_names"
}

run_build_only_reaper_test() {
  # Building the runtime installs process-global context that cannot be reset.
  # Keep this real build-path regression in a fresh process so it cannot narrow
  # the DomainSet observed by later registry and domain tests.
  cargo_test --lib \
    "agent::tinyagents::reaper::tests::a_build_only_runtime_is_swept_before_it_can_be_invoked" \
    -- --exact --test-threads=1 "$@"
}

run_tinychannels_linked_fixture_tests() {
  # These confidential calls use the linked test module and run in fresh
  # processes so process-wide module-loader state cannot leak into the unit
  # suite. The linked source is explicit and never falls back to downloading.
  OPENHUMAN_MODULE_PATH="$tinychannels_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}" \
    cargo_test --features e2e-test-support --lib channels::module_fixture:: \
      -- --test-threads=1 "$@"
  OPENHUMAN_MODULE_PATH="$tinychannels_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}" \
    cargo_test --features e2e-test-support --lib \
      modules::ops::tests::a_registered_linked_factory_serves_the_module_and_status_says_so \
      -- --exact --test-threads=1 "$@"
  OPENHUMAN_MODULE_PATH="$tinychannels_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}" \
    cargo_test --lib \
    channels::host::channel_events::tests:: -- --test-threads=1 "$@"
}

run_full_suite() {
  # Several unit fixtures mutate process-wide state (provider overrides and
  # temporary executable paths). Keep this aggregate invocation deterministic;
  # integration targets below retain their own, narrower isolation strategies.
  OPENHUMAN_MODULE_PATH="$connectors_module_path" \
    cargo_test --lib -- --test-threads=1 \
    --skip a_build_only_runtime_is_swept_before_it_can_be_invoked "$@"
  # The core binary and the developer bins live in `crates/openhuman-cli`;
  # `--workspace --bins` picks them up there (plus the TUI binary).
  cargo_test --bins -- --test-threads=1 "$@"
  run_build_only_reaper_test "$@"
  run_tinychannels_linked_fixture_tests "$@"
  cargo_test --doc -- "$@"

  while IFS= read -r target; do
    [ -n "$target" ] || continue
    if [ "$target" = "tinydocs_linked_e2e" ]; then
      # This local-only target is intentionally outside the product set; turn
      # on its linked implementation feature explicitly so the glob does not
      # silently skip it or Cargo reject it for missing required features.
      cargo_test --features linked-modules --test "$target" -- "$@"
    elif [ "$target" = "raw_coverage_all" ]; then
      # These suites used to run as separate integration-test binaries. Run
      # each generated module filter in its own cargo process so local
      # `pnpm test:rust` preserves the same process-global isolation as CI.
      run_raw_coverage_modules "$@"
    elif [ "$target" = "in_process_all" ]; then
      run_in_process_modules "$@"
    elif [ "$target" = "json_rpc_e2e" ]; then
      run_json_rpc_e2e "$@"
    elif [ "$target" = "tinymcp_storage_smoke_e2e" ]; then
      OPENHUMAN_MODULE_PATH="$tinymcp_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}" \
        cargo_test --features e2e-test-support --test "$target" -- "$@"
    elif [ "$target" = "mcp_registry_e2e" ]; then
      # The TinyMCP module owns process-level directory transitions; this
      # binary exercises several workspaces against that shared module object.
      OPENHUMAN_MODULE_PATH="$connectors_module_path" \
        cargo_test --test "$target" -- --ignored --test-threads=1 "$@"
    elif [ "$target" = "mcp_registry_multi_server" ]; then
      # The host opens multiple per-workspace clients to one TinyMCP module.
      # Keep this integration target serial so those module-backed transitions
      # do not race through the process-global loader.
      OPENHUMAN_TINYMCP_TEST_MODULE="$tinymcp_artifact" \
        cargo_test --test "$target" -- --test-threads=1 "$@"
    else
      OPENHUMAN_MODULE_PATH="$connectors_module_path" \
        cargo_test --test "$target" -- "$@"
    fi
  done < <(integration_test_targets)

  # Loading a native module mutates process-global module state, so exercise
  # the published contract in a fresh libtest process with its pinned artifact.
  OPENHUMAN_MODULE_PATH="$hosts_dir:$connectors_module_path" \
    cargo_test --lib \
      hosting::test::released_artifact_serves_providers_and_confidential_local_preparation \
      -- --exact --ignored --test-threads=1
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
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "mcp_registry_e2e" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  OPENHUMAN_MODULE_PATH="$connectors_module_path" \
    cargo_test --test mcp_registry_e2e -- --ignored --test-threads=1 "$@"
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "tinymcp_storage_smoke_e2e" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  OPENHUMAN_MODULE_PATH="$tinymcp_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}" \
    cargo_test --features e2e-test-support --test tinymcp_storage_smoke_e2e -- "$@"
elif [ "$#" -ge 2 ] && [ "$1" = "--test" ] && [ "$2" = "mcp_registry_multi_server" ]; then
  shift 2
  if [ "${1:-}" = "--" ]; then
    shift
  fi
  OPENHUMAN_TINYMCP_TEST_MODULE="$tinymcp_artifact" \
    cargo_test --test mcp_registry_multi_server -- --test-threads=1 "$@"
else
  cargo_test "$@"
fi
