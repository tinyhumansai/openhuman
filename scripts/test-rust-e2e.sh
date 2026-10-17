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
HAS_EXTRA_ARGS=0
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
      if [ "$#" -gt 0 ]; then
        HAS_EXTRA_ARGS=1
      fi
      break
      ;;
    *)
      EXTRA_ARGS+=("$1")
      HAS_EXTRA_ARGS=1
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
CREATED_XDG_DATA_HOME=""
CREATED_XDG_CACHE_HOME=""
XDG_ROOT="${OPENHUMAN_XDG_ROOT:-${TMPDIR:-/tmp}}"

# TinyBus rejects module-search paths under a shared runner HOME mount. Keep
# module state created by this test runner in an owner-only scratch directory.
if [ -z "${XDG_DATA_HOME:-}" ]; then
  XDG_DATA_HOME="$(mktemp -d "${XDG_ROOT}/openhuman-rust-e2e-xdg.XXXXXX")"
  chmod 700 "$XDG_DATA_HOME"
  export XDG_DATA_HOME
  CREATED_XDG_DATA_HOME="$XDG_DATA_HOME"
fi

# dirs::cache_dir uses XDG_CACHE_HOME rather than XDG_DATA_HOME. Give it a
# separate private root: the release cache must not also be scanned as a
# TinyBus module search path.
if [ -z "${XDG_CACHE_HOME:-}" ]; then
  XDG_CACHE_HOME="$(mktemp -d "${XDG_ROOT}/openhuman-rust-e2e-cache.XXXXXX")"
  chmod 700 "$XDG_CACHE_HOME"
  CREATED_XDG_CACHE_HOME="$XDG_CACHE_HOME"
  export XDG_CACHE_HOME
fi

cleanup() {
  if [ -n "$MOCK_PID" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
  if [ -n "$CREATED_XDG_DATA_HOME" ]; then
    rm -rf "$CREATED_XDG_DATA_HOME" 2>/dev/null || true
  fi
  if [ -n "$CREATED_XDG_CACHE_HOME" ]; then
    rm -rf "$CREATED_XDG_CACHE_HOME" 2>/dev/null || true
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

# The Linux/macOS CI host may not provide a usable Secret Service. Supply a
# per-run encryption key so the core's encrypted storage backend can persist
# the E2E session without depending on the host keyring. Never print the key.
if [ -z "${OPENHUMAN_KEYRING_MASTER_KEY:-}" ] && [ -z "${OPENHUMAN_KEYRING_MASTER_KEY_FILE:-}" ]; then
  OPENHUMAN_KEYRING_MASTER_KEY="$(openssl rand -hex 32)"
  export OPENHUMAN_KEYRING_MASTER_KEY
fi

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
# scripts/e2e-core.sh adds a storage driver (storage-mongodb) the product set lacks.
if [ -n "${OPENHUMAN_E2E_EXTRA_FEATURES:-}" ]; then
  PRODUCT_FEATURES="$PRODUCT_FEATURES,$OPENHUMAN_E2E_EXTRA_FEATURES"
fi

module_target="$(bash "$SCRIPT_DIR/ci/native-test-module-platform.sh")"
IFS=$'\t' read -r module_ext module_platform <<< "$module_target"
# shellcheck source=modules/stage-local-modules.sh
source "$SCRIPT_DIR/modules/stage-local-modules.sh"
# With OPENHUMAN_MODULE_SOURCE=source the modules below are built from the pinned
# submodules (scripts/modules/build.mjs); otherwise TinyBox and TinyMCP are the
# registry-pinned releases, as in CI.
e2e_source_ids=()
if local_module_source_is_source; then
  e2e_source_ids+=(tinyconnectors tinysearch tinybox tinymcp)
elif [ "${OPENHUMAN_TEST_MODULES_STAGED:-}" = "1" ]; then
  echo "[rust-e2e] Using pre-staged native modules from OPENHUMAN_MODULE_PATH."
else
  # Shell command analysis and the storage URL E2E suites load TinyBox through
  # the native module path; the ignored MCP registry cases need TinyMCP. Both
  # come from the registry-pinned release, digest-verified before any
  # subprocess starts.
  for released_id in tinyconnectors tinyflows tinysearch tinybox tinymcp; do
    released_dir="$(stage_release_module "$released_id" "$module_platform" "$module_ext")"
    export OPENHUMAN_MODULE_PATH="$released_dir${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
  done
fi

# Module-backed Composio coverage must use a staged artifact as well. Without
# it the core resolves TinyConnectors through release metadata at run time,
# turning an otherwise hermetic mock-backend suite into a network dependency
# and permanently faulting that process when the lookup fails. The isolated
# module-admission suite likewise needs a real native search module. The
# released archives are staged above (digest-verified); only the local
# OPENHUMAN_MODULE_SOURCE=source mode builds anything, through build.mjs.
if [ "${#e2e_source_ids[@]}" -gt 0 ]; then
  echo "[rust-e2e] Building pinned local test modules: ${e2e_source_ids[*]} ..."
  stage_source_modules "${e2e_source_ids[@]}"
fi
for source_id in ${e2e_source_ids[@]+"${e2e_source_ids[@]}"}; do
  export OPENHUMAN_MODULE_PATH="$REPO_ROOT/target/module-sources/$source_id${OPENHUMAN_MODULE_PATH:+:$OPENHUMAN_MODULE_PATH}"
done

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
      --exact --test-threads=1 ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
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
    if [ "$module" = "config_auth_app_state_connectivity_e2e" ]; then
      # These RPC fixtures share process-global credential and listener state;
      # serialize them so one test cannot drop another test's ephemeral server.
      bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
        --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
        --test in_process_all -- "${module}::" --test-threads=1 \
        ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
      continue
    fi
    if [ "$module" = "domain_modules_e2e" ]; then
      # TinyFlows and TinyMCP use process-global module runtimes. A different
      # test's RPC host teardown can leave a later module call unavailable, so
      # run module-backed cases in fresh processes and keep the remaining
      # domain tests together in their normal harness.
      local -a isolated_module_tests=(
        domain_modules_e2e::cron_update_applies_a_partial_patch_without_clobbering_unset_fields
        domain_modules_e2e::mcp_audit_list_dispatches_internally_and_stays_out_of_the_schema_dump
        domain_modules_e2e::mcp_clients_read_paths_validate_before_reaching_outward
        domain_modules_e2e::mcp_tool_description_scan_flags_injection_and_spares_ordinary_prose
      )
      local -a skip_module_tests=()
      local test_name
      for test_name in "${isolated_module_tests[@]}"; do
        skip_module_tests+=(--skip "$test_name")
      done
      bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
        --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
        --test in_process_all -- "${module}::" --test-threads=1 \
        "${skip_module_tests[@]}" ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
      for test_name in "${isolated_module_tests[@]}"; do
        echo "[rust-e2e]     isolated test: ${test_name}"
        bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
          --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
          --test in_process_all -- "$test_name" --exact ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
      done
      continue
    fi
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
  if [ "$suite" = "mcp_registry_e2e" ]; then
    # The registry tests open several process-global module clients across
    # workspaces. Parallel libtest threads can race their module lifecycle.
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite (serial)"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES" \
      --test "$suite" -- --ignored --test-threads=1 ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
    continue
  fi
  if [ "$suite" = "tinymcp_storage_smoke_e2e" ]; then
    # Released TinyMCP cdylib under the real host and its storage backend
    # (#7737). The staging above put it on OPENHUMAN_MODULE_PATH. It spawns
    # openhuman-core twice, so run it serially.
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite (serial)"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES,e2e-test-support" \
      --test "$suite" -- --test-threads=1 ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
    continue
  fi
  if [ "$suite" = "tinydocs_linked_e2e" ]; then
    # This local-only E2E target is outside the product feature set. Enable
    # its required linked implementation explicitly instead of failing Cargo's
    # required-features check.
    echo "[rust-e2e]   $CARGO_BIN test --manifest-path Cargo.toml --test $suite (linked-modules)"
    bash "$SCRIPT_DIR/ci-cancel-aware.sh" "$CARGO_BIN" test \
      --manifest-path Cargo.toml --features "$PRODUCT_FEATURES,linked-modules" \
      --test "$suite" -- ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"}
    continue
  fi

  if [ "$HAS_EXTRA_ARGS" -eq 1 ]; then
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
