#!/usr/bin/env bash
# PR CI Rust coverage lane.
#
# Every Rust-core change runs the complete product-feature test suite. Path
# filtering decides whether this high-level area runs; it never narrows the
# suite to changed files or source modules.
set -euo pipefail

OUT="${OUT:-lcov-core.info}"
PRODUCT_FEATURES="$(bash scripts/ci/product-features.sh)"
CORE_PRODUCT_FEATURES="$(bash scripts/ci/product-features.sh --core)"

# Sandboxed acting-tool tests deliberately do not forward RUSTUP_HOME. The
# container installs Rust outside HOME, so its rustup proxy otherwise tries
# to create ~/.rustup inside the write-confined jail. Put the already selected
# toolchain binaries first; cargo --version then needs no rustup home writes.
# Keep cargo-installed subcommands on PATH after the toolchain directory.
if command -v rustup >/dev/null 2>&1; then
  if COV_TOOLCHAIN_CARGO="$(rustup which cargo 2>/dev/null)"; then
    export PATH="$(dirname "$COV_TOOLCHAIN_CARGO"):$PATH"
  fi
fi

log() { echo "[ci][rust-cov] $*"; }

# cargo-llvm-cov owns RUSTFLAGS while it instruments crates. Keeping the
# job-level linker flag here can suppress instrumentation and produce no data.
unset RUSTFLAGS

llvm_cov() {
  case "${1:-}" in
    clean | report)
      bash scripts/ci-cancel-aware.sh cargo llvm-cov "$@"
      ;;
    *)
      if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
        bash scripts/ci/nextest-coverage.sh "${COV_SUITE:-coverage}" cargo llvm-cov nextest --profile ci --no-tests=fail \
          --features "${CORE_PRODUCT_FEATURES}" "$@"
      else
        bash scripts/ci-cancel-aware.sh cargo llvm-cov \
          --features "${CORE_PRODUCT_FEATURES}" "$@"
      fi
      ;;
  esac
}

llvm_cov_package() {
  if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
    bash scripts/ci/nextest-coverage.sh "${COV_SUITE:-coverage}" cargo llvm-cov nextest --profile ci --no-tests=fail "$@"
  else
    bash scripts/ci-cancel-aware.sh cargo llvm-cov "$@"
  fi
}

llvm_cov_product_facade() {
  if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
    bash scripts/ci/nextest-coverage.sh "${COV_SUITE:-coverage}" cargo llvm-cov nextest --profile ci --no-tests=fail \
      --features "${CORE_PRODUCT_FEATURES}" "$@"
  else
    bash scripts/ci-cancel-aware.sh cargo llvm-cov \
      --features "${CORE_PRODUCT_FEATURES}" "$@"
  fi
}

llvm_cov_embed() {
  llvm_cov_product_facade "$@"
}

# The nextest runner builds every product-feature package with ONE Cargo
# argument set. Cargo hashes each unit by its resolved feature set, and the
# core's feature graph differs per selected package set (core alone,
# embed/tinyhumans, and the cli, which drops the core's `default` alias), so
# separate `-p` invocations each compile the core and its dependencies again.
# The packages run as one suite whose filter is the union of what the old
# per-package arguments built and ran:
#   core        its only target is the lib (`--lib --bins`)
#   embed       `--all-targets`
#   tinyhumans  `--all-targets`
#   cli         `--tests -E 'kind(test)'` (integration targets only)
UNIFIED_PRODUCT_ARGS=(-p openhuman -p openhuman-embed -p openhuman-tinyhumans -p openhuman-cli --all-targets)
FILTER_CORE='package(=openhuman)'
FILTER_EMBED='package(=openhuman-embed)'
FILTER_TINYHUMANS='package(=openhuman-tinyhumans)'
FILTER_CLI_TESTS='package(=openhuman-cli) & kind(test)'
FILTER_PRODUCT="${FILTER_CORE} | ${FILTER_EMBED} | ${FILTER_TINYHUMANS} | (${FILTER_CLI_TESTS})"

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

in_process_modules() {
  find tests/in_process -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/in_process/##' -e 's#\.rs$##' |
    sort
}

# Print each explicitly declared integration-test target and its required
# features as "<name><TAB><comma-separated gates>".
test_target_required_features() {
  awk '
    /^\[\[test\]\]/ { if (name != "" && req != "") print name "\t" req; name=""; req=""; inblk=1; next }
    /^\[/              { if (name != "" && req != "") print name "\t" req; name=""; req=""; inblk=0 }
    inblk && /^name[ \t]*=/ {
      line=$0; sub(/^name[ \t]*=[ \t]*"/, "", line); sub(/".*$/, "", line); name=line; next
    }
    inblk && /^required-features[ \t]*=/ {
      line=$0
      sub(/^required-features[ \t]*=[ \t]*\[/, "", line); sub(/\].*$/, "", line)
      gsub(/[" ]/, "", line); req=line; next
    }
    END { if (name != "" && req != "") print name "\t" req }
  ' crates/openhuman-cli/Cargo.toml
}

TEST_TARGET_REQS="$(test_target_required_features)"

target_features_satisfied() {
  local target="$1" req feature
  req="$(printf '%s\n' "${TEST_TARGET_REQS}" | awk -F'\t' -v t="${target}" '$1 == t { print $2 }')"
  [ -n "${req}" ] || return 0
  for feature in $(printf '%s' "${req}" | tr ',' ' '); do
    case ",${PRODUCT_FEATURES}," in
      *",${feature},"*) ;;
      *) return 1 ;;
    esac
  done
}

# These target-level cfgs intentionally produce empty binaries in this build.
# Keep exceptions named and narrow: every other expected target must report
# tests, including newly added targets. The Composio stress case runs once in
# the private uninstrumented lane; existing Windows checks remain separate.
target_expected_in_coverage() {
  local target="$1" platform="${2:-$(uname -s)}"
  case "$target" in
    composio_list_tools_stack_overflow_regression)
      log "not expecting ${target} in coverage: #![cfg(not(coverage))]; tested uninstrumented"
      return 1
      ;;
    windows_sandbox_env_e2e)
      case "$platform" in
        MINGW* | MSYS* | CYGWIN* | Windows_NT) ;;
        *)
          log "not expecting ${target} on ${platform}: all tests are #[cfg(windows)]"
          return 1
          ;;
      esac
      ;;
  esac
  return 0
}

# Every suite runs even when an earlier one fails, so one red crate never
# hides the results of the crates and targets after it. Failures are
# collected and reported together at the end, after the lcov report is still
# merged from whatever did run. A cancellation (130/143 from
# scripts/ci-cancel-aware.sh) stops the run at once.
FAILED_SUITES=()

# Cargo names why it rebuilds a unit ("Dirty <crate>: <reason>") only in
# verbose mode. Keep those lines, in the log and in ci-out/rebuilds/<suite>.log,
# and drop the verbose noise ("Running `rustc ...`", "Fresh <crate>"): an
# identical suite rebuilding the whole graph (the embed suite after the core
# suite in EX63 runs 39365491098 and 39371566398) is otherwise unexplained.
# OH_COV_EXPLAIN_REBUILDS=0 turns it off.
explain_rebuilds() {
  awk -v reasons="$1" '
    /^ +Dirty / { print >> reasons; print; fflush(); next }
    /^ +(Running `|Fresh )/ { next }
    { print; fflush() }
  '
}

suite() {
  local name="$1" rc=0 slug
  shift
  slug="$(printf '%s' "$name" | tr -cs '[:alnum:]' '-')"
  if [ "${OH_COV_EXPLAIN_REBUILDS:-1}" = "1" ]; then
    mkdir -p ci-out/rebuilds
    CARGO_TERM_VERBOSE=true COV_SUITE="${slug}" "$@" 2> >(explain_rebuilds "ci-out/rebuilds/${slug}.log" >&2) || rc=$?
  else
    COV_SUITE="${slug}" "$@" || rc=$?
  fi
  # On the self-hosted guest (CI_SCRATCH_DIR is set) log what the scratch disk
  # has left and how large the coverage target and its raw profiles are after
  # each suite, so a full disk (EX63 run 39292268284) names the suite that
  # grew it.
  if [ -n "${CI_SCRATCH_DIR:-}" ]; then
    log "disk after ${name}: $(df -h "${CI_SCRATCH_DIR}" 2>/dev/null | awk 'NR==2 { print $4 " free of " $2 }')," \
      "target $(du -sh "${CARGO_BUILD_TARGET_DIR:-target}" 2>/dev/null | cut -f1)," \
      "profraw $(find "${CARGO_BUILD_TARGET_DIR:-target}" -name '*.profraw' -print0 2>/dev/null | du -ch --files0-from=- 2>/dev/null | tail -1 | cut -f1)"
  fi
  case "${rc}" in
    0) ;;
    130 | 143)
      log "cancelled during ${name} (exit ${rc})"
      exit "${rc}"
      ;;
    *)
      FAILED_SUITES+=("${name} (exit ${rc})")
      log "FAILED: ${name} (exit ${rc}); continuing with the remaining suites"
      ;;
  esac
}

run_integration_target() {
  local target="$1"
  if ! target_features_satisfied "${target}"; then
    log "skipping ${target}: required features are not in the product set"
    return 0
  fi

  if [ "${target}" = "raw_coverage_all" ]; then
    # These modules mutate process-global state. Keep one process per module
    # while paying the link cost for the aggregate target only once.
    while IFS= read -r module; do
      [ -n "${module}" ] || continue
      log "running raw coverage module: ${module}"
      suite "${target}::${module}" llvm_cov --no-report --no-fail-fast -p openhuman-cli \
        --test "${target}" -- "${module}::" --test-threads=1
    done < <(raw_coverage_modules)
  elif [ "${target}" = "in_process_all" ]; then
    # The former in-process router targets share one binary; keep one process
    # per module so their process globals stay per suite, as before.
    while IFS= read -r module; do
      [ -n "${module}" ] || continue
      log "running in-process module: ${module}"
      suite "${target}::${module}" llvm_cov --no-report --no-fail-fast -p openhuman-cli \
        --test "${target}" -- "${module}::"
    done < <(in_process_modules)
  elif [ "${target}" = "mcp_registry_multi_server" ]; then
    # Its cases share TinyMCP's process-wide module client; keep them serial,
    # as scripts/test-rust-with-mock.sh does.
    suite "${target}" llvm_cov --no-report --no-fail-fast -p openhuman-cli \
      --test "${target}" -- --test-threads=1
  elif [[ "${target}" =~ ^json_rpc_.*_e2e$ ]]; then
    # JSON-RPC tests share runtime/config globals and must remain serial.
    suite "${target}" llvm_cov --no-report --no-fail-fast -p openhuman-cli \
      --test "${target}" -- --test-threads=1
  else
    suite "${target}" llvm_cov --no-report --no-fail-fast -p openhuman-cli --test "${target}"
  fi
}

run_build_only_reaper_test() {
  suite "openhuman reaper (isolated)" llvm_cov --no-report --no-fail-fast -p openhuman --lib -- \
    agent::tinyagents::reaper::tests::a_build_only_runtime_is_swept_before_it_can_be_invoked \
    --exact --test-threads=1
}

if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
  # One process per test means one .profraw per process by default (the name
  # carries %p): ~11,800 files and ~50 GB for the core alone, which the report
  # then spends minutes merging. Without %p, `%8m` makes LLVM merge each
  # process's counters online into a pool of 8 files per instrumented binary as
  # it exits. cargo-llvm-cov reads the pattern from LLVM_PROFILE_FILE_NAME.
  export LLVM_PROFILE_FILE_NAME="${LLVM_PROFILE_FILE_NAME:-openhuman-%8m.profraw}"
  # The harness retries a failing model call on a 3/6/12/24 s schedule (about
  # 45 s per turn). Suites that drive an unreachable or erroring upstream sat
  # through it on a worker slot doing nothing: 9 of the 10 slowest tests in the
  # warm EX63 run (60-90 s each) were that wait. The same schedule, with the
  # waits scaled to 100 ms, keeps every retry and every assertion. The
  # schedule itself is pinned by a unit test that ignores this variable.
  export OPENHUMAN_MODEL_RETRY_BACKOFF_MS="${OPENHUMAN_MODEL_RETRY_BACKOFF_MS:-100}"
fi

log "running complete instrumented Rust suite (runner: ${OH_COV_RUNNER:-cargo})"
llvm_cov clean --workspace

# The storage-host cdylib tests load a real fixture module. Left to the tests,
# every test process (nextest runs one per test) runs its own `cargo build` of
# it and queues on the fixture target's lock: 14 tests at ~30-40 s each. Build
# it once, uninstrumented exactly as the tests would, and hand them the path.
if [ -z "${OPENHUMAN_STORAGE_PROBE_MODULE:-}" ]; then
  probe_target="${PWD}/target/storage-probe"
  log "building the storage probe module once into ${probe_target}"
  env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS -u RUSTC_WRAPPER -u CARGO_TARGET_DIR \
    bash scripts/ci-cancel-aware.sh cargo build --quiet \
    --manifest-path tests/fixtures/storage_probe_module/Cargo.toml \
    --target-dir "${probe_target}"
  case "$(uname -s)" in
    Darwin) probe_file="libstorage_probe_module.dylib" ;;
    MINGW* | MSYS* | CYGWIN*) probe_file="storage_probe_module.dll" ;;
    *) probe_file="libstorage_probe_module.so" ;;
  esac
  export OPENHUMAN_STORAGE_PROBE_MODULE="${probe_target}/debug/${probe_file}"
fi

# Keep the private invocations sequential in the configured coverage target.
# Each package retains its own feature resolution; instrumentation is merged
# once after all suites, including successful suites before a failure.
COVERAGE_REPORTED=0
preserve_coverage() {
  local status=$?
  trap - EXIT INT TERM HUP
  if [ "${COVERAGE_REPORTED}" = "0" ]; then
    log "preserving partial coverage into ${OUT} (exit ${status})"
    CI_CANCEL_WATCHDOG=0 llvm_cov report --lcov --output-path "${OUT}" || \
      log "partial coverage report unavailable; retaining raw profiles"
  fi
  exit "${status}"
}
trap preserve_coverage EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
  # The core, embed, tinyhumans and cli suites are one nextest run: one build
  # of the product graph and one test run over the union of their filters.
  # Run as four invocations with the same arguments, Cargo still rebuilt most
  # of the graph between them (5-6 min each in EX63 runs 39365491098 and
  # 39371566398), and their tests could not overlap. Every integration target
  # still runs in its own processes, one per test, and the listing below is
  # the same build, so the run that follows finds it fresh.
  mkdir -p ci-out/nextest
  expected_targets="${PWD}/ci-out/nextest/cli-expected-targets.txt"
  : >"${expected_targets}"
  while IFS= read -r target; do
    [ -n "${target}" ] || continue
    if target_features_satisfied "${target}" && target_expected_in_coverage "${target}"; then
      printf '%s\n' "${target}" >>"${expected_targets}"
    fi
  done < <(integration_test_targets)
  expected_inventory="${PWD}/ci-out/nextest/product-inventory.json"
  # Listing executes no tests. cargo-llvm-cov supplies the same wrapper and
  # instrumentation environment, but show-env leaves Cargo on the plain
  # target dir while `cargo llvm-cov nextest` builds in its llvm-cov-target
  # subdir. Without the explicit --target-dir the listing re-instrumented
  # the whole graph a second time (578 crates, +53 GB on the EX63 scratch
  # disk in run 39315551099, which then filled it). Listing also launches
  # each instrumented binary; keep those profiles out of the report.
  product_inventory() (
    inventory_env="$(bash scripts/ci-cancel-aware.sh cargo llvm-cov show-env --sh)" || exit $?
    eval "${inventory_env}"
    inventory_profiles="${PWD}/ci-out/nextest/product-inventory-profraw"
    export LLVM_PROFILE_FILE="${inventory_profiles}/list-%p.profraw"
    bash scripts/ci-cancel-aware.sh cargo nextest list --message-format json --features "${CORE_PRODUCT_FEATURES}" \
      --target-dir "${CARGO_LLVM_COV_TARGET_DIR}/llvm-cov-target" \
      "${UNIFIED_PRODUCT_ARGS[@]}" -E "${FILTER_PRODUCT}" >"${expected_inventory}"
    rm -rf "${inventory_profiles}"
  )
  suite "openhuman product test inventory" product_inventory
  NEXTEST_EXPECTED_TARGETS="${expected_targets}" \
  NEXTEST_EXPECTED_INVENTORY="${expected_inventory}" \
  suite "openhuman product suites (nextest)" llvm_cov --no-report --no-fail-fast \
    "${UNIFIED_PRODUCT_ARGS[@]}" -E "${FILTER_PRODUCT}"
else
  # Keep the aggregate unit-test process aligned with the canonical Rust runner.
  # The isolated reaper test installs one-shot process globals, so it cannot run
  # in the same process as the rest of the library tests.
  suite "openhuman lib+bins" llvm_cov --no-report --no-fail-fast -p openhuman --lib --bins -- \
    --test-threads=1 \
    --skip a_build_only_runtime_is_swept_before_it_can_be_invoked

  log "running isolated build-only reaper test"
  run_build_only_reaper_test
fi

# Run every root-workspace Rust support crate rather than only crates named by
# changed paths. Product features are forwarded to the embedding facade; the
# remaining crates do not expose that feature vocabulary.
if [ "${OH_COV_RUNNER:-cargo}" != "nextest" ]; then
  suite "openhuman-embed" llvm_cov_embed --no-report --no-fail-fast -p openhuman-embed --all-targets
fi
suite "openhuman-rpc" llvm_cov_package --no-report --no-fail-fast -p openhuman-rpc --all-targets \
  --features storage-sqlite
if [ "${OH_COV_RUNNER:-cargo}" != "nextest" ]; then
  suite "openhuman-tinyhumans" llvm_cov_product_facade --no-report --no-fail-fast -p openhuman-tinyhumans --all-targets
fi
if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
  # Contributor/default features are distinct from product features. Keep both
  # configurations instrumented rather than dropping the default-only surface.
  # Embed and tinyhumans run as one suite: one build of the default-feature
  # core instead of two. Embed alone resolves its graph exactly as the pair
  # does; tinyhumans gains only the two features embed's dev-dependencies turn
  # on (tinymemory-api `conformance`, tinystoragedrivers-document `sqlite`),
  # the same unification the product suite above already accepts.
  suite "openhuman-embed + openhuman-tinyhumans (default)" llvm_cov_package --no-report --no-fail-fast \
    -p openhuman-embed -p openhuman-tinyhumans --all-targets
fi
# The terminal frontend builds the core a third time (default features, not the
# product set). It is on by default and on in the PR lane; OH_COV_TUI=0 is an
# opt-out for local runs.
if [ "${OH_COV_TUI:-1}" = "1" ]; then
  suite "openhuman-tui" llvm_cov_package --no-report --no-fail-fast -p openhuman-tui --all-targets
else
  log "skipping openhuman-tui (OH_COV_TUI=${OH_COV_TUI})"
fi

if [ "${OH_COV_RUNNER:-cargo}" != "nextest" ]; then
  # The nextest runner ran the integration targets with the product suites.
  while IFS= read -r target; do
    [ -n "${target}" ] || continue
    log "running integration target: ${target}"
    run_integration_target "${target}"
  done < <(integration_test_targets)
fi

# Doctests are not collected by cargo-llvm-cov, but they are still part of the
# complete Rust test suite and must run whenever the Rust-core area changes.
# They compile their own uninstrumented core, so a caller that runs them in a
# parallel lane instead (CI Fast on the EX63) sets OH_COV_DOCTESTS=0.
if [ "${OH_COV_DOCTESTS:-1}" = "1" ]; then
  suite "openhuman doctests" bash scripts/ci-cancel-aware.sh cargo test -p openhuman \
    --doc --features "${PRODUCT_FEATURES}"
else
  log "skipping doctests (OH_COV_DOCTESTS=${OH_COV_DOCTESTS}); the caller runs them"
fi

log "merging coverage into ${OUT}"
suite "lcov report" llvm_cov report --lcov --output-path "${OUT}"

COVERAGE_REPORTED=1

# A full product build must produce records for every eligible source file,
# except in the crates whose suites this run skipped (OH_COV_TUI=0).
presence_skip=""
[ "${OH_COV_TUI:-1}" = "1" ] || presence_skip="crates/openhuman-tui/src/"
[ -z "${presence_skip}" ] || log "coverage presence: not checking ${presence_skip} (suite skipped)"
suite "coverage presence" env COVERAGE_PRESENCE_SKIP_PREFIXES="${presence_skip}" \
  bash scripts/ci/assert-coverage-presence.sh "${OUT}" --all

if [ "${#FAILED_SUITES[@]}" -gt 0 ]; then
  log "${#FAILED_SUITES[@]} suite(s) failed:"
  printf '[ci][rust-cov]   - %s\n' "${FAILED_SUITES[@]}"
  exit 1
fi
log "all suites passed"
