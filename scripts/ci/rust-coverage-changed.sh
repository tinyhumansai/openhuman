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
# TWO GATES GUARD THE "WE VERIFIED NOTHING" CASE (PR #5593):
#   1. scripts/ci/assert-coverage-presence.sh — hard failure when a changed
#      source file produced no lcov records at all, i.e. the lane never
#      compiled it. This is the precise one; it names the files.
#   2. A zero-executed-tests scoped run ESCALATES to the full suite rather than
#      failing. Scoping that selects no tests is unsafe scoping, and this
#      script's standing policy for unsafe scoping is to widen, not to redden —
#      a domain that legitimately owns no unit tests (there are five today,
#      e.g. core::shutdown) must not turn every PR touching it red.
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

# The CI job normally supplies a linker-only RUSTFLAGS value. cargo-llvm-cov
# owns this variable while it compiles coverage-instrumented crates; preserving
# the outer value suppresses its `-C instrument-coverage` flag and leaves an
# otherwise successful test run with no .profraw data to report.
unset RUSTFLAGS

llvm_cov() {
  # `clean` and `report` are cargo-llvm-cov subcommands that take no feature
  # selection; passing --features to them is an error.
  case "${1:-}" in
    clean | report)
      bash scripts/ci-cancel-aware.sh cargo llvm-cov "$@"
      return
      ;;
  esac
  # Let cargo-llvm-cov own compiler instrumentation and raw-profile
  # collection. A hand-exported `show-env` setup can be bypassed by the
  # repository's Cargo wrapper configuration in container jobs.
  bash scripts/ci-cancel-aware.sh cargo llvm-cov --features "${PRODUCT_FEATURES}" "$@"
}

# Total libtest cases executed across every scoped/full run in this invocation.
# `run_counted` tees libtest output so the count can be read without changing
# what the log looks like. `${PIPESTATUS[0]}` — not `$?` — carries the cargo
# exit status through the pipe; reading `$?` here would report tee's status and
# turn a failing suite green.
TESTS_RUN=0

run_counted() {
  local log rc n
  log="$(mktemp)"
  set +e
  "$@" 2>&1 | tee "${log}"
  rc=${PIPESTATUS[0]}
  set -e
  n="$(sed -n 's/^running \([0-9]\{1,\}\) tests\{0,1\}$/\1/p' "${log}" | awk '{s+=$1} END {print s+0}')"
  TESTS_RUN=$((TESTS_RUN + n))
  rm -f "${log}"
  return "${rc}"
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
#   src/openhuman/agent/harness/session/** and src/openhuman/threads/goals/**
#   → `agent_turn_overrides_e2e`. Per-turn `TurnOverrides` (`session/types.rs`)
#   are consumed in `session/turn/core_turn.rs`, and the terminal thread-goal
#   APIs live in `threads/goals/runtime.rs`; the whole contract is an
#   integration target, so without this a regression in either could merge
#   through CI Lite having executed none of those assertions. Scoped to the two
#   directories the suite actually guards rather than all of `agent/**`, which
#   would drag this target onto most PRs in the tree for no added signal.
#
# Echoes zero or more target names, one per line; the caller tolerates an
# empty result.
domain_integration_targets() {
  case "$1" in
    src/openhuman/memory/*)
      printf '%s\n' memory_golden_fixture_e2e memory_golden_parity_e2e
      ;;
    src/openhuman/agent/harness/session/* | src/openhuman/threads/goals/*)
      printf '%s\n' agent_turn_overrides_e2e
      ;;
  esac
}

raw_coverage_modules() {
  find tests/raw_coverage -maxdepth 1 -type f -name '*.rs' -print |
    sed -e 's#^tests/raw_coverage/##' -e 's#\.rs$##' |
    sort
}

# `required-features` of each `[[test]]` target in Cargo.toml, as
# "<name><TAB><comma-separated gates>". Targets without the key are omitted.
#
# Parsed from Cargo.toml rather than `cargo metadata` so this stays a
# dependency-free awk/bash script (no jq, no python) on bash 3.2 and 5.x alike.
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
  ' Cargo.toml
}

TEST_TARGET_REQS="$(test_target_required_features)"

# True when every `required-features` gate of ${1} is enabled in PRODUCT_FEATURES.
#
# **Why this guard exists.** `cargo` only SKIPS a target for unsatisfied
# `required-features` when the target is selected IMPLICITLY (a bare
# `cargo test`). Every call site here names the target explicitly
# (`--test <name>`), and naming an unsatisfiable target is a hard ERROR:
#
#     error: target `memory_artifacts_e2e` in package `openhuman`
#            requires the features: `memory-git`
#
# That never fired while every `required-features` gate happened to be in the
# product set. Dropping `memory-git` from the product set made
# `memory_artifacts_e2e` the first unsatisfiable one and took this whole lane
# down — on a PR that had nothing wrong with it. Skipping here restores the
# behaviour the `required-features` line was written to express, and keeps the
# next gate removal from breaking the lane the same way.
target_features_satisfied() {
  local target="$1" req f
  req="$(printf '%s\n' "${TEST_TARGET_REQS}" | awk -F'\t' -v t="${target}" '$1 == t { print $2 }')"
  [ -n "${req}" ] || return 0
  for f in $(printf '%s' "${req}" | tr ',' ' '); do
    case ",${PRODUCT_FEATURES}," in
      *",${f},"*) ;;
      *) return 1 ;;
    esac
  done
  return 0
}

run_integration_target() {
  local target="$1"
  if ! target_features_satisfied "${target}"; then
    log "skipping ${target}: required-features not in the product set"
    return 0
  fi
  if [ "${target}" = "raw_coverage_all" ]; then
    # These suites used to be separate integration-test binaries. Aggregating
    # them removes repeated full-crate links, but many still exercise process
    # globals (env vars, event bus handlers, auth tokens, singleton stores).
    # Run one process per generated module filter to preserve the former
    # per-binary isolation contract while still paying only one link.
    #
    # `|| return` is load-bearing, not defensive noise. This loop's exit status
    # is that of its LAST iteration, so a module that fails followed by one that
    # succeeds reports success. That used to be masked by ambient errexit — the
    # function was called bare, so a failing `llvm_cov` aborted the script here.
    # It is no longer: `run_counted` runs its command inside a pipeline with
    # `set +e`, which disables errexit for everything underneath, so without this
    # the failure is silently discarded and a red suite goes green.
    #
    # Returning on the first failure also preserves the previous fail-fast
    # timing exactly: no module ran after a failure before, and none does now.
    while IFS= read -r module; do
      [ -n "${module}" ] || continue
      log "running raw coverage module: ${module}"
      llvm_cov --no-report --no-fail-fast -p openhuman --test "${target}" -- "${module}::" --test-threads=1 || return
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

compile_raw_coverage_target() {
  log "compiling raw coverage integration target for src/** change"
  bash scripts/ci-cancel-aware.sh cargo test \
    --features "${PRODUCT_FEATURES}" \
    --test raw_coverage_all --no-run
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
  # FULL mode has no changed-file list (the workflow blanks CHANGED_FILES to
  # stay under the container's argv limit), so assert the whole-tree invariant
  # instead: no eligible source file may be missing from a full product build's
  # coverage. This is the mode PR #5578 ran in when it first landed the
  # uncompiled hosting family, and it is the mode that would have caught it.
  bash scripts/ci/assert-coverage-presence.sh "${OUT}" --all
  exit 0
}

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

src_changed=false

if [ "${#files[@]}" -eq 0 ]; then
  run_full "empty changed-file list — scoping unsafe"
fi
if [ "${#files[@]}" -gt "${MAX_CHANGED_FILES}" ]; then
  run_full "${#files[@]} changed files exceed MAX_CHANGED_FILES=${MAX_CHANGED_FILES}"
fi

lib_filters_raw=""
test_targets_raw=""
for f in "${files[@]}"; do
  case "${f}" in
    src/*) src_changed=true ;;
  esac
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
      # Standalone ops/bench binaries have no domain unit tests to scope to.
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

if [ "${src_changed}" = true ]; then
  # Scoped lib tests cannot compile integration targets that are not selected by
  # a domain mapping. Build the aggregate raw-coverage target on every src/**
  # change so source-only PRs cannot leave a broken integration suite behind.
  compile_raw_coverage_target
fi

# Drop artifacts from previous coverage runs so merged profdata only reflects
# this run (build cache for dependencies is unaffected).
llvm_cov clean --workspace

if [ "${#lib_filters[@]}" -gt 0 ]; then
  log "running scoped lib unit tests with filters: ${lib_filters[*]}"
  # libtest ORs multiple positional filters — one run covers all domains.
  run_counted llvm_cov --no-report --no-fail-fast -p openhuman --lib -- "${lib_filters[@]}"
fi

if [ "${#test_targets[@]}" -gt 0 ]; then
  for t in "${test_targets[@]}"; do
    log "running changed integration-test target: ${t}"
    run_counted run_integration_target "${t}"
  done
fi

log "merging coverage into ${OUT}"
llvm_cov report --lcov --output-path "${OUT}"

# Gate 1 (precise, hard): did the lane produce ANY coverage records for the
# files this PR changed? Run before the zero-test escalation so the hosting-class
# defect — a file the build never compiled — fails in ~10 minutes with the file
# names, instead of first spending ~40 minutes on a full suite that cannot
# compile it either.
bash scripts/ci/assert-coverage-presence.sh "${OUT}" --files "${files[@]}"

# Gate 2 (imprecise, safe): a scoped run that executed no tests verified
# nothing. Widen rather than fail — see the header note.
if [ "${TESTS_RUN}" -eq 0 ]; then
  log "scoped run executed 0 tests (filters: ${lib_filters[*]-none}; targets: ${test_targets[*]-none})"
  run_full "scoped run executed 0 tests — scoping selected no coverage"
fi
