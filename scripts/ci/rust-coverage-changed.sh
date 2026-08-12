#!/usr/bin/env bash
# PR CI Rust core coverage lane — changed-files-only cargo-llvm-cov.
#
# Fast-lane policy (PRs targeting main): instead of the full ~13k-test
# instrumented suite, run only the unit tests for the modules the PR touched:
#   - src/<a>/<b>/... .rs  → libtest filter "<a>::<b>" (domain-level scope, so
#     sibling-module tests like store_tests.rs / ops.rs still run)
#   - tests/<name>.rs      → that integration-test target only (--test <name>)
# On top of that, a small table (`domain_integration_targets`) drags in the
# integration targets that GUARD a domain but live outside `--lib`, so a PR
# touching only that domain's src/ still runs its gate.
# Coverage from all scoped runs is merged (--no-report + report) into a single
# lcov file; the PR CI Gate's diff-cover step enforces >= 80% on changed lines.
#
# NOTE: this means changed lines must be covered by tests in their own domain
# (or a changed integration test) — coverage contributed by unrelated suites
# no longer counts on the fast lane. The full suite still runs on main→release
# PRs (Release CI).
#
# Inputs (env):
#   FULL          "true" → run the full suite (build-config / lib.rs / script
#                 changes, detected by paths-filter)
#   CHANGED_FILES shell-quoted, space-separated repo-relative paths from
#                 dorny/paths-filter (list-files: shell)
#   OUT           lcov output path (default lcov-core.info)
#
# Falls back to the FULL suite whenever scoping is not clearly safe.
set -euo pipefail

FULL="${FULL:-false}"
CHANGED_FILES="${CHANGED_FILES:-}"
OUT="${OUT:-lcov-core.info}"
MAX_CHANGED_FILES="${MAX_CHANGED_FILES:-200}"

log() { echo "[ci][rust-cov-changed] $*"; }

# The desktop product's gates. `[features] default` is the CONTRIBUTOR set now
# and deliberately omits voice, web3, documents, meet, contacts, inference and
# crash-reporting — so a coverage run on default features would silently stop
# measuring code that ships, and the diff-coverage gate would pass a PR whose
# changed lines were never compiled. Source of truth:
# scripts/ci/product-features.txt.
PRODUCT_FEATURES="$(bash scripts/ci/product-features.sh)"

llvm_cov() {
  # `clean` and `report` are cargo-llvm-cov subcommands that take no feature
  # selection; passing --features to them is an error.
  case "${1:-}" in
    clean | report | show-env)
      bash scripts/ci-cancel-aware.sh cargo llvm-cov "$@"
      return
      ;;
  esac
  # `show-env` above configures ordinary Cargo test commands for coverage.
  # Invoking cargo-llvm-cov again under those exported variables is unsupported
  # (and fails before tests run), so remove its report-only flag and run Cargo.
  local -a cargo_args=()
  local arg
  for arg in "$@"; do
    [ "${arg}" = "--no-report" ] && continue
    cargo_args+=("${arg}")
  done
  bash scripts/ci-cancel-aware.sh cargo test --features "${PRODUCT_FEATURES}" "${cargo_args[@]}"
}

integration_test_targets() {
  find tests -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/##' -e 's#\.rs$##' |
    sort
}

# Integration-test targets a changed *source* path must drag in, on top of its
# `--lib` filter.
#
# The default scoping maps `src/<a>/<b>/…` to the libtest filter `<a>::<b>`,
# which runs `--lib` only. That is right for domains whose contract is unit
# tested, and wrong for domains whose contract lives in an integration target:
# such a gate never runs on a PR that touches only the domain's `src/`.
#
#   src/openhuman/memory/** → the golden-workspace schema gates. They stand
#   between a memory-store schema change and a corrupted user workspace, and
#   they are `tests/` targets, so `--lib` scoping alone skips them entirely.
#
# Echoes zero or more target names, one per line; the caller tolerates an
# empty result.
domain_integration_targets() {
  case "$1" in
    src/openhuman/memory/*)
      printf '%s\n' memory_golden_fixture_e2e memory_golden_parity_e2e
      ;;
  esac
}

raw_coverage_modules() {
  find tests/raw_coverage -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/raw_coverage/##' -e 's#\.rs$##' |
    sort
}

run_integration_target() {
  local target="$1"
  if [ "${target}" = "raw_coverage_all" ]; then
    # These suites used to be separate integration-test binaries. Aggregating
    # them removes repeated full-crate links, but many still exercise process
    # globals (env vars, event bus handlers, auth tokens, singleton stores).
    # Run one process per generated module filter to preserve the former
    # per-binary isolation contract while still paying only one link.
    while IFS= read -r module; do
      [ -n "${module}" ] || continue
      log "running raw coverage module: ${module}"
      llvm_cov --no-report --no-fail-fast -p openhuman --test "${target}" -- "${module}::" --test-threads=1
    done < <(raw_coverage_modules)
  elif [ "${target}" = "json_rpc_e2e" ]; then
    # This target exercises process-global runtime/config state. Its tests take
    # an environment lock, but background agent tasks can outlive an individual
    # case briefly; keeping libtest serial prevents a successor from observing
    # that teardown window.
    llvm_cov --no-report --no-fail-fast -p openhuman --test "${target}" -- --test-threads=1
  else
    llvm_cov --no-report --no-fail-fast -p openhuman --test "${target}"
  fi
}

run_full() {
  log "running FULL instrumented suite (reason: $1)"
  llvm_cov clean --workspace
  llvm_cov --no-report --no-fail-fast -p openhuman --lib
  llvm_cov --no-report --no-fail-fast -p openhuman --bins
  while IFS= read -r target; do
    [ -n "${target}" ] || continue
    log "running full-suite integration target: ${target}"
    run_integration_target "${target}"
  done < <(integration_test_targets)
  log "merging coverage into ${OUT}"
  llvm_cov report --lcov --output-path "${OUT}"
  exit 0
}

# Containerised GitHub runners mount the workspace from the host. LLVM can run
# the test binary successfully there while silently failing to create its raw
# profile on that mount. `show-env` is cargo-llvm-cov's supported mode for
# instrumenting ordinary Cargo test commands; override only its profile output
# after it has configured the compiler wrapper.
eval "$(cargo llvm-cov show-env --sh)"
# Keep profiles where cargo-llvm-cov itself reports from. This avoids relying
# on bind-mounted temporary paths that the report subprocess cannot observe.
profile_dir="${LLVM_PROFILE_DIR:-${CARGO_LLVM_COV_TARGET_DIR}}"
mkdir -p "${profile_dir}"
export LLVM_PROFILE_FILE="${profile_dir}/core-%p-%m.profraw"

if [ "${FULL}" = "true" ]; then
  run_full "build-config/workflow-level change detected by paths-filter"
fi

# Portable across bash 3.2 (macOS) and 5.x (CI containers): no declare -A,
# no mapfile, and no empty-array "${arr[@]}" expansion under set -u.
#
# CHANGED_FILES is the shell-quoted list from dorny/paths-filter
# (list-files: shell). Filenames are PR-controlled, so never eval it —
# xargs unquotes tokens as data without ever invoking a shell. If xargs
# can't parse it (e.g. hostile quoting), we get an empty list and fall
# back to the full suite.
declare -a files=()
while IFS= read -r f; do
  [ -n "${f}" ] && files+=("${f}")
done < <(printf '%s\n' "${CHANGED_FILES}" | xargs -n1 printf '%s\n' 2>/dev/null || true)
log "received ${#files[@]} changed rust file(s)"

if [ "${#files[@]}" -eq 0 ]; then
  run_full "empty changed-file list — scoping unsafe"
fi
if [ "${#files[@]}" -gt "${MAX_CHANGED_FILES}" ]; then
  run_full "${#files[@]} changed files exceed MAX_CHANGED_FILES=${MAX_CHANGED_FILES}"
fi

lib_filters_raw=""
test_targets_raw=""
for f in "${files[@]}"; do
  if [ ! -e "${f}" ]; then
    # dorny/paths-filter includes deleted paths. They contain no changed lines
    # to cover and, for tests, no longer correspond to runnable Cargo targets.
    log "ignoring deleted rust-relevant path: ${f}"
    continue
  fi
  case "${f}" in
    src/lib.rs | src/main.rs)
      run_full "root module ${f} changed — whole-crate scope"
      ;;
    src/bin/*)
      # Standalone backfill binaries (slack-backfill, gmail-backfill-3d) have
      # no unit tests; nothing to scope to.
      log "ignoring standalone-binary file: ${f}"
      ;;
    src/*.rs)
      p="${f#src/}"
      p="${p%.rs}"
      IFS='/' read -r -a segs <<<"${p}"
      n="${#segs[@]}"
      if [ "${segs[n - 1]}" = "mod" ]; then
        segs=("${segs[@]:0:n-1}")
        n="${#segs[@]}"
      fi
      if [ "${n}" -ge 2 ]; then
        key="${segs[0]}::${segs[1]}"
      else
        key="${segs[0]}"
      fi
      lib_filters_raw="${lib_filters_raw}${key}
"
      log "${f} → libtest filter '${key}'"
      while IFS= read -r extra_target; do
        [ -n "${extra_target}" ] || continue
        test_targets_raw="${test_targets_raw}${extra_target}
"
        log "${f} → integration gate '--test ${extra_target}'"
      done < <(domain_integration_targets "${f}")
      ;;
    src/*/*)
      # Non-.rs asset embedded in a domain (e.g. agent prompt markdown under
      # src/openhuman/agent/prompts/) — scope to that domain's tests.
      p="${f#src/}"
      IFS='/' read -r -a segs <<<"${p}"
      n="${#segs[@]}"
      if [ "${n}" -ge 3 ]; then
        key="${segs[0]}::${segs[1]}"
      else
        key="${segs[0]}"
      fi
      lib_filters_raw="${lib_filters_raw}${key}
"
      log "${f} → libtest filter '${key}' (embedded asset)"
      while IFS= read -r extra_target; do
        [ -n "${extra_target}" ] || continue
        test_targets_raw="${test_targets_raw}${extra_target}
"
        log "${f} → integration gate '--test ${extra_target}'"
      done < <(domain_integration_targets "${f}")
      ;;
    tests/fixtures/memory_golden/*)
      # The golden memory-workspace fixture (committed .db blobs + the derived
      # manifest). A change here IS the schema-gate re-baseline, so run the
      # gates rather than falling through to the `*)` full-suite arm.
      test_targets_raw="${test_targets_raw}memory_golden_fixture_e2e
"
      log "${f} → integration gate '--test memory_golden_fixture_e2e'"
      ;;
    tests/raw_coverage/*.rs)
      # The ~76 *_raw_coverage_e2e.rs suites are aggregated into the single
      # `raw_coverage_all` target (see tests/raw_coverage_all.rs + build.rs), so
      # a change to any of them scopes to that one target rather than the full
      # suite. libtest filters within the aggregate binary still work, but the
      # simplest correct scope is running the whole aggregate target.
      test_targets_raw="${test_targets_raw}raw_coverage_all
"
      log "${f} → aggregated integration target '--test raw_coverage_all'"
      ;;
    tests/*.rs)
      name="${f#tests/}"
      name="${name%.rs}"
      if [[ "${name}" == */* ]]; then
        # Nested support module — can affect any integration target.
        run_full "shared integration-test support file ${f} changed"
      fi
      test_targets_raw="${test_targets_raw}${name}
"
      log "${f} → integration target '--test ${name}'"
      ;;
    *)
      run_full "unclassified rust-relevant file ${f} changed"
      ;;
  esac
done

declare -a lib_filters=()
while IFS= read -r k; do
  [ -n "${k}" ] && lib_filters+=("${k}")
done < <(printf '%s' "${lib_filters_raw}" | sort -u)

declare -a test_targets=()
while IFS= read -r k; do
  [ -n "${k}" ] && test_targets+=("${k}")
done < <(printf '%s' "${test_targets_raw}" | sort -u)

if [ "${#lib_filters[@]}" -eq 0 ] && [ "${#test_targets[@]}" -eq 0 ]; then
  run_full "no scoped test targets derivable from the change set"
fi

# Drop artifacts from previous coverage runs so merged profdata only reflects
# this run (build cache for dependencies is unaffected).
llvm_cov clean --workspace

if [ "${#lib_filters[@]}" -gt 0 ]; then
  log "running scoped lib unit tests with filters: ${lib_filters[*]}"
  # libtest ORs multiple positional filters — one run covers all domains.
  llvm_cov --no-report --no-fail-fast -p openhuman --lib -- "${lib_filters[@]}"
fi

if [ "${#test_targets[@]}" -gt 0 ]; then
  for t in "${test_targets[@]}"; do
    log "running changed integration-test target: ${t}"
    run_integration_target "${t}"
  done
fi

shopt -s nullglob
profiles=("${profile_dir}"/*.profraw)
test "${#profiles[@]}" -gt 0

log "merging coverage into ${OUT}"
llvm_cov report --lcov --output-path "${OUT}"
