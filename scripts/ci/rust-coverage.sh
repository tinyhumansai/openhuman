#!/usr/bin/env bash
# PR CI Rust coverage lane.
#
# Every Rust-core change runs the complete product-feature test suite. Path
# filtering decides whether this high-level area runs; it never narrows the
# suite to changed files or source modules.
set -euo pipefail

OUT="${OUT:-lcov-core.info}"
PRODUCT_FEATURES="$(bash scripts/ci/product-features.sh)"

log() { echo "[ci][rust-cov] $*"; }

# cargo-llvm-cov owns RUSTFLAGS while it instruments crates. Keeping the
# job-level linker flag here can suppress instrumentation and produce no data.
unset RUSTFLAGS

# mold without touching RUSTFLAGS: `mold -run` intercepts the linker exec.
CARGO=(cargo)
if command -v mold >/dev/null 2>&1; then
  CARGO=(mold -run cargo)
fi

llvm_cov() {
  case "${1:-}" in
    clean | report)
      bash scripts/ci-cancel-aware.sh "${CARGO[@]}" llvm-cov "$@"
      ;;
    *)
      bash scripts/ci-cancel-aware.sh "${CARGO[@]}" llvm-cov \
        --features "${PRODUCT_FEATURES}" "$@"
      ;;
  esac
}

llvm_cov_package() {
  bash scripts/ci-cancel-aware.sh "${CARGO[@]}" llvm-cov "$@"
}

package_features() {
  local list="" pkg feature
  for pkg in "$@"; do
    for feature in $(printf '%s' "${PRODUCT_FEATURES}" | tr ',' ' '); do
      list="${list:+${list},}${pkg}/${feature}"
    done
  done
  printf '%s\n' "${list}"
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

prebuild_integration_targets() {
  log "prebuilding integration targets"
  (
    eval "$(cargo llvm-cov show-env --export-prefix)"
    bash scripts/ci-cancel-aware.sh "${CARGO[@]}" test --no-run \
      -p openhuman-cli --tests --features "${PRODUCT_FEATURES}"
  )
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
      llvm_cov --no-report --no-fail-fast -p openhuman-cli \
        --test "${target}" -- "${module}::" --test-threads=1 || return
    done < <(raw_coverage_modules)
  elif [ "${target}" = "json_rpc_e2e" ]; then
    # JSON-RPC tests share runtime/config globals and must remain serial.
    llvm_cov --no-report --no-fail-fast -p openhuman-cli \
      --test "${target}" -- --test-threads=1
  else
    llvm_cov --no-report --no-fail-fast -p openhuman-cli --test "${target}"
  fi
}

run_tinyjuice_regression() {
  if [ -z "${TINYJUICE_TEST_MODULE:-}" ]; then
    log "TINYJUICE_TEST_MODULE unset — skipping TinyJuice host-module regression"
    return 0
  fi
  log "running TinyJuice host-module regression"
  llvm_cov --no-report -p openhuman --lib -- \
    openhuman::agent::tinyagents::middleware::tests::tool_output_tabulates_a_large_graph_for_a_non_exempt_tool \
    --ignored --exact
}

log "running complete instrumented Rust suite"
llvm_cov clean --workspace

# Keep the aggregate unit-test process aligned with the canonical Rust runner.
# The isolated reaper test installs one-shot process globals, so it cannot run
# in the same process as the rest of the library tests.
llvm_cov --no-report --no-fail-fast -p openhuman --lib --bins -- \
  --test-threads=1 \
  --skip a_build_only_runtime_is_swept_before_it_can_be_invoked

log "running isolated build-only reaper test"
llvm_cov --no-report --no-fail-fast -p openhuman --lib -- \
  openhuman::agent::tinyagents::reaper::tests::a_build_only_runtime_is_swept_before_it_can_be_invoked \
  --exact --test-threads=1

run_tinyjuice_regression

log "running support crate suites"
llvm_cov_package --no-report --no-fail-fast \
  -p openhuman-embed -p openhuman-tinyhumans -p openhuman-rpc -p openhuman-tui \
  --all-targets \
  --features "$(package_features openhuman-embed openhuman-tinyhumans)"

prebuild_integration_targets

while IFS= read -r target; do
  [ -n "${target}" ] || continue
  log "running integration target: ${target}"
  run_integration_target "${target}"
done < <(integration_test_targets)


log "merging coverage into ${OUT}"
llvm_cov report --lcov --output-path "${OUT}"

# A full product build must produce records for every eligible source file.
bash scripts/ci/assert-coverage-presence.sh "${OUT}" --all
