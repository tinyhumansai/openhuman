#!/usr/bin/env bash
# PR CI Rust coverage lane.
#
# Every Rust-core change runs the complete product-feature test suite. Path
# filtering decides whether this high-level area runs; it never narrows the
# suite to changed files or source modules.
set -euo pipefail

OUT="${OUT:-lcov-core.info}"
PRODUCT_FEATURES="$(bash scripts/ci/product-features.sh)"

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
      bash scripts/ci-cancel-aware.sh cargo llvm-cov \
        --features "${PRODUCT_FEATURES}" "$@"
      ;;
  esac
}

llvm_cov_package() {
  bash scripts/ci-cancel-aware.sh cargo llvm-cov "$@"
}

llvm_cov_product_facade() {
  bash scripts/ci-cancel-aware.sh cargo llvm-cov \
    --features "${PRODUCT_FEATURES}" "$@"
}

llvm_cov_embed() {
  (
    # Embed's integration hosts persist fixture credentials with the real
    # encrypted backend. Supply only this suite a disposable master key;
    # core's missing-key regressions and the caller's environment stay intact.
    unset OPENHUMAN_KEYRING_MASTER_KEY_FILE
    OPENHUMAN_KEYRING_MASTER_KEY="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
    export OPENHUMAN_KEYRING_MASTER_KEY
    llvm_cov_product_facade "$@"
  )
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

# Every suite runs even when an earlier one fails, so one red crate never
# hides the results of the crates and targets after it. Failures are
# collected and reported together at the end, after the lcov report is still
# merged from whatever did run. A cancellation (130/143 from
# scripts/ci-cancel-aware.sh) stops the run at once.
FAILED_SUITES=()
suite() {
  local name="$1" rc=0
  shift
  "$@" || rc=$?
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
  elif [ "${target}" = "json_rpc_e2e" ]; then
    # JSON-RPC tests share runtime/config globals and must remain serial.
    suite "${target}" llvm_cov --no-report --no-fail-fast -p openhuman-cli \
      --test "${target}" -- --test-threads=1
  else
    suite "${target}" llvm_cov --no-report --no-fail-fast -p openhuman-cli --test "${target}"
  fi
}

if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
  # One process per test means one .profraw per process by default (the name
  # carries %p): ~11,800 files and ~50 GB for the core alone, which the report
  # then spends minutes merging. Without %p, `%8m` makes LLVM merge each
  # process's counters online into a pool of 8 files per instrumented binary as
  # it exits. cargo-llvm-cov reads the pattern from LLVM_PROFILE_FILE_NAME.
  export LLVM_PROFILE_FILE_NAME="${LLVM_PROFILE_FILE_NAME:-openhuman-%8m.profraw}"
fi

log "running complete instrumented Rust suite (runner: ${OH_COV_RUNNER:-cargo})"
llvm_cov clean --workspace

UNIT_PID=""
if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
  # cargo-nextest runs every test in its own process, in parallel. Process
  # globals (the event bus, registries, env vars, one-shot OnceLocks) are then
  # per test, so the serial runner and the isolated reaper run below are not
  # needed: every test is isolated. Settings: .config/nextest.toml [profile.ci].
  #
  # The core's unit tests compile the core as a test program; every other suite
  # below compiles it as a library. Neither build needs the other, so the unit
  # tests run in the background in a target dir of their own (one cargo per
  # target dir; a shared one would serialize them on its lock) and write their
  # own lcov file, beside this run's. Each invocation keeps exactly the package
  # selection, and so the feature resolution, it has when run alone.
  UNIT_TARGET_DIR="${CARGO_TARGET_DIR:-${PWD}/target}/cov-unit"
  UNIT_OUT="${OUT%.info}-unit.info"
  UNIT_LOG="${OUT%.info}-unit.log"
  UNIT_STATUS="${OUT%.info}-unit.status"
  rm -f "${UNIT_OUT}" "${UNIT_STATUS}"
  (
    export CARGO_TARGET_DIR="${UNIT_TARGET_DIR}"
    rc=0
    llvm_cov clean --workspace || rc=$?
    if [ "${rc}" = 0 ]; then
      llvm_cov nextest --profile ci --no-report --no-fail-fast -p openhuman --lib --bins || rc=$?
    fi
    case "${rc}" in 130 | 143) ;; *)
      report_rc=0
      llvm_cov report --lcov --output-path "${UNIT_OUT}" || report_rc=$?
      [ "${rc}" != 0 ] || rc="${report_rc}"
      ;;
    esac
    echo "${rc}" >"${UNIT_STATUS}"
  ) >"${UNIT_LOG}" 2>&1 &
  UNIT_PID=$!
  log "core unit tests started in the background (pid ${UNIT_PID}, target ${UNIT_TARGET_DIR}, log ${UNIT_LOG})"
else
  # Keep the aggregate unit-test process aligned with the canonical Rust runner.
  # The isolated reaper test installs one-shot process globals, so it cannot run
  # in the same process as the rest of the library tests.
  suite "openhuman lib+bins" llvm_cov --no-report --no-fail-fast -p openhuman --lib --bins -- \
    --test-threads=1 \
    --skip a_build_only_runtime_is_swept_before_it_can_be_invoked

  log "running isolated build-only reaper test"
  suite "openhuman reaper (isolated)" llvm_cov --no-report --no-fail-fast -p openhuman --lib -- \
    openhuman::agent::tinyagents::reaper::tests::a_build_only_runtime_is_swept_before_it_can_be_invoked \
    --exact --test-threads=1
fi

# Run every root-workspace Rust support crate rather than only crates named by
# changed paths. Product features are forwarded to the embedding facade; the
# remaining crates do not expose that feature vocabulary.
suite "openhuman-embed" llvm_cov_embed --no-report --no-fail-fast -p openhuman-embed --all-targets
suite "openhuman-rpc" llvm_cov_package --no-report --no-fail-fast -p openhuman-rpc --all-targets
suite "openhuman-tinyhumans" llvm_cov_product_facade --no-report --no-fail-fast -p openhuman-tinyhumans --all-targets
# The terminal frontend builds the core a third time (default features, not the
# product set). It is on by default and on in the PR lane; OH_COV_TUI=0 is an
# opt-out for local runs.
if [ "${OH_COV_TUI:-1}" = "1" ]; then
  suite "openhuman-tui" llvm_cov_package --no-report --no-fail-fast -p openhuman-tui --all-targets
else
  log "skipping openhuman-tui (OH_COV_TUI=${OH_COV_TUI})"
fi

if [ "${OH_COV_RUNNER:-cargo}" = "nextest" ]; then
  # Every integration target in one parallel run, one process per test: the
  # raw_coverage modules and the JSON-RPC tests are then isolated from each
  # other without the per-module and serial invocations below. Cargo skips a
  # target whose required-features are not in the product set; `kind(test)`
  # keeps the run to the integration targets, as the loop below runs them.
  suite "openhuman-cli integration tests (nextest)" llvm_cov nextest --profile ci \
    --no-report --no-fail-fast -p openhuman-cli --tests -E 'kind(test)'
else
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

presence_lcov="${OUT}"
if [ -n "${UNIT_PID}" ]; then
  log "waiting for the core unit tests (pid ${UNIT_PID})"
  wait "${UNIT_PID}" || true
  echo "::group::[ci][rust-cov] core unit tests (nextest, background)"
  cat "${UNIT_LOG}"
  echo "::endgroup::"
  unit_rc="$(cat "${UNIT_STATUS}" 2>/dev/null || echo 1)"
  suite "openhuman lib+bins (nextest)" test "${unit_rc}" = 0 || true
  case "${unit_rc}" in 130 | 143) exit "${unit_rc}" ;; esac
  # One file for the presence gate; diff-cover reads both lcov files itself.
  presence_lcov="${OUT%.info}.presence"
  cat "${OUT}" "${UNIT_OUT}" >"${presence_lcov}" 2>/dev/null || true
fi

# A full product build must produce records for every eligible source file,
# except in the crates whose suites this run skipped (OH_COV_TUI=0).
presence_skip=""
[ "${OH_COV_TUI:-1}" = "1" ] || presence_skip="crates/openhuman-tui/src/"
[ -z "${presence_skip}" ] || log "coverage presence: not checking ${presence_skip} (suite skipped)"
suite "coverage presence" env COVERAGE_PRESENCE_SKIP_PREFIXES="${presence_skip}" \
  bash scripts/ci/assert-coverage-presence.sh "${presence_lcov}" --all
[ "${presence_lcov}" = "${OUT}" ] || rm -f "${presence_lcov}"

if [ "${#FAILED_SUITES[@]}" -gt 0 ]; then
  log "${#FAILED_SUITES[@]} suite(s) failed:"
  printf '[ci][rust-cov]   - %s\n' "${FAILED_SUITES[@]}"
  exit 1
fi
log "all suites passed"
