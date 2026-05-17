#!/usr/bin/env bash
# scripts/test_install.sh — smoke-tests the install.sh resolver in isolation.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Use a fixture latest.json that mirrors what the real release publishes.
FIXTURE="$REPO_ROOT/scripts/fixtures/latest.json"

# The resolver function should be sourced, not invoked end-to-end (no curl).
if ! source "$REPO_ROOT/scripts/install.sh" --source-only 2>/dev/null; then
  echo "FAIL: scripts/install.sh does not support --source-only mode"
  exit 1
fi

resolved=$(resolve_asset_url "$FIXTURE" "linux" "x86_64")
expected="https://example.invalid/openhuman_0.0.0-test_amd64.AppImage"
if [[ "$resolved" != "$expected" ]]; then
  echo "FAIL: expected $expected, got $resolved"
  exit 1
fi

# Also test a missing platform produces exit code 3.
set +e
resolve_asset_url "$FIXTURE" "linux" "aarch64" >/dev/null 2>&1
missing_platform_rc=$?
set -e
if [[ "$missing_platform_rc" -ne 3 ]]; then
  echo "FAIL: expected exit code 3 for missing platform linux-aarch64, got $missing_platform_rc"
  exit 1
fi

assert_retry_shape() {
  local calls="$1" label="$2"
  local _ first second extra
  IFS='|' read -r _ first second extra <<<"${calls}"

  if [[ -z "${first:-}" || -z "${second:-}" || -n "${extra:-}" ]]; then
    echo "FAIL: ${label} should issue exactly 2 curl calls (base + HTTP/1.1 retry)"
    exit 1
  fi

  if [[ "${first}" == *"--http1.1"* || "${second}" != *"--http1.1"* ]]; then
    echo "FAIL: ${label} should retry with --http1.1 only on the second call"
    exit 1
  fi
}

(
  CURL_CALLS=""
  curl() {
    CURL_CALLS="${CURL_CALLS}|$*"
    case " $* " in
      *" --http1.1 "*) return 0 ;;
      *) return 16 ;;
    esac
  }

  if ! curl_head_with_http_fallback "https://example.invalid/OpenHuman.app.tar.gz"; then
    echo "FAIL: reachability fallback should succeed when HTTP/1.1 retry succeeds"
    exit 1
  fi
  assert_retry_shape "${CURL_CALLS}" "reachability check"
)

(
  CURL_CALLS=""
  curl() {
    CURL_CALLS="${CURL_CALLS}|$*"
    case " $* " in
      *" --http1.1 "*) return 0 ;;
      *) return 16 ;;
    esac
  }

  if ! curl_get_file "https://example.invalid/latest.json" "/tmp/openhuman-test-latest.json"; then
    echo "FAIL: metadata fetch fallback should succeed when HTTP/1.1 retry succeeds"
    exit 1
  fi
  assert_retry_shape "${CURL_CALLS}" "metadata fetch"
)

(
  CURL_CALLS=""
  curl() {
    CURL_CALLS="${CURL_CALLS}|$*"
    case " $* " in
      *" --http1.1 "*) return 0 ;;
      *) return 16 ;;
    esac
  }

  if ! curl_download_file "https://example.invalid/OpenHuman.app.tar.gz" "/tmp/openhuman-test-download"; then
    echo "FAIL: download fallback should succeed when HTTP/1.1 retry succeeds"
    exit 1
  fi
  assert_retry_shape "${CURL_CALLS}" "download"
)

echo "PASS"
