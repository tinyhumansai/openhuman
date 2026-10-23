#!/usr/bin/env bash
# Exercise the released module through the compiled registry and archive digest.
# For explicit local module development, set OPENHUMAN_TEST_SECURITY_MODULE.
set -euo pipefail

if [[ -z "${OPENHUMAN_TEST_SECURITY_MODULE:-}" ]]; then
  case "$(uname -s)" in
    Darwin)
      case "$(uname -m)" in
        arm64) security_host="macos-15-arm64" ;;
        x86_64) security_host="macos-15-x86_64" ;;
        *) echo "Unsupported macOS architecture" >&2; exit 1 ;;
      esac ;;
    Linux)
      case "$(uname -m)" in
        aarch64) security_host="ubuntu-22.04-arm64" ;;
        x86_64) security_host="ubuntu-22.04-x86_64" ;;
        *) echo "Unsupported Linux architecture" >&2; exit 1 ;;
      esac ;;
    MINGW* | MSYS* | CYGWIN*) security_host="windows-2025-x86_64" ;;
    *) echo "Unsupported native security fixture host" >&2; exit 1 ;;
  esac
  export OPENHUMAN_TEST_SECURITY_RELEASE_HOST="${OPENHUMAN_TEST_SECURITY_RELEASE_HOST:-$security_host}"
fi

export OPENHUMAN_TEST_SECURITY_FIXTURE_DIR="$HOME/.cache/openhuman-security-fixtures"
mkdir -p "$OPENHUMAN_TEST_SECURITY_FIXTURE_DIR"

# Release-build latency gate. With OPENHUMAN_SECURITY_PERF_BUDGET=1 the same
# native test runs under --release and fails when the per-call p50 or p99 passes
# the owner's committed budget (p50 <= 1 ms, p99 <= 10 ms; see
# vendor/tinysecurity/docs/performance.md). The measured record, including the
# calls per representative turn, is written to OPENHUMAN_SECURITY_PERF_OUT.
set --
if [[ "${OPENHUMAN_SECURITY_PERF_BUDGET:-}" == "1" ]]; then
  set -- --release
  export OPENHUMAN_SECURITY_PERF_BUDGET
  export OPENHUMAN_SECURITY_PERF_OUT="${OPENHUMAN_SECURITY_PERF_OUT:-$PWD/target/security-perf.json}"
  mkdir -p "$(dirname "$OPENHUMAN_SECURITY_PERF_OUT")"
fi

for test_filter in \
  modules::security \
  security::approval \
  security::egress \
  security::redact \
  sandbox::spawn_plan \
  tools::implementations; do
  bash scripts/ci-cancel-aware.sh cargo test -p openhuman --lib "$@" \
    --no-default-features \
    --features inference,web3,modules,security-module,native-security-tests,flows,storage-sqlite,secret-file,legacy-import \
    "$test_filter" -- --nocapture
done

# Production clients use the process-wide module runtime, which intentionally
# has no shutdown/unload path. Run each such test in a fresh process so its
# loaded native module cannot race the isolated-broker fixture tests above.
for test_filter in \
  modules::security::production_tests::production_client_registers_and_checks_a_path_policy_on_the_released_archive \
  modules::security::production_tests::sync_seam_redacts_through_the_released_archive_from_inside_and_outside_a_runtime \
  modules::security::production_tests::production_client_gates_shell_and_network_on_the_released_archive \
  modules::security::production_tests::production_keyring_is_denied_and_creates_no_store_on_the_released_archive; do
  bash scripts/ci-cancel-aware.sh cargo test -p openhuman --lib "$@" \
    --no-default-features \
    --features inference,web3,modules,security-module,native-security-tests,flows,storage-sqlite,secret-file,legacy-import \
    "$test_filter" -- --ignored --nocapture
done
