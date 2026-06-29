#!/usr/bin/env bash
# scripts/test_install.sh — smoke-tests the install.sh resolver in isolation.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Use a fixture latest.json that mirrors what the real release publishes.
FIXTURE="$REPO_ROOT/scripts/fixtures/latest.json"
RELEASE_FIXTURE="$REPO_ROOT/scripts/fixtures/release.json"

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

resolved_arm64=$(resolve_asset_url "$FIXTURE" "linux" "aarch64")
expected_arm64="https://example.invalid/openhuman_0.0.0-test_arm64.AppImage"
if [[ "$resolved_arm64" != "$expected_arm64" ]]; then
  echo "FAIL: expected $expected_arm64, got $resolved_arm64"
  exit 1
fi

release_parsed=$(resolve_release_asset_metadata "$RELEASE_FIXTURE" "linux" "x86_64" "deb")
release_name=$(echo "$release_parsed" | sed -n '2p')
release_url=$(echo "$release_parsed" | sed -n '3p')
release_digest=$(echo "$release_parsed" | sed -n '4p')
if [[ "$release_name" != "OpenHuman_0.0.0-test_amd64.deb" ]]; then
  echo "FAIL: Debian/Ubuntu linux x86_64 should prefer .deb, got $release_name"
  exit 1
fi
if [[ "$release_url" != "https://example.invalid/OpenHuman_0.0.0-test_amd64.deb" ]]; then
  echo "FAIL: expected .deb URL, got $release_url"
  exit 1
fi
if [[ "$release_digest" != "deb-amd64" ]]; then
  echo "FAIL: expected .deb digest, got $release_digest"
  exit 1
fi

release_arm64_parsed=$(resolve_release_asset_metadata "$RELEASE_FIXTURE" "linux" "aarch64" "deb")
release_arm64_name=$(echo "$release_arm64_parsed" | sed -n '2p')
if [[ "$release_arm64_name" != "OpenHuman_0.0.0-test_arm64.deb" ]]; then
  echo "FAIL: Debian/Ubuntu linux aarch64 should prefer .deb, got $release_arm64_name"
  exit 1
fi

release_appimage_parsed=$(resolve_release_asset_metadata "$RELEASE_FIXTURE" "linux" "x86_64" "appimage")
release_appimage_name=$(echo "$release_appimage_parsed" | sed -n '2p')
if [[ "$release_appimage_name" != "OpenHuman_0.0.0-test_amd64.AppImage" ]]; then
  echo "FAIL: AppImage linux x86_64 should still resolve AppImage, got $release_appimage_name"
  exit 1
fi

if ! declare -F install_linux >/dev/null; then
  echo "FAIL: scripts/install.sh --source-only should expose install_linux for dry-run coverage"
  exit 1
fi

(
  HOME="$(mktemp -d)"
  trap 'rm -rf "$HOME"' EXIT
  OS=linux
  ARCH=x86_64
  ASSET_NAME="OpenHuman_0.0.0-test_amd64.deb"
  DOWNLOAD_PATH="/tmp/OpenHuman_0.0.0-test_amd64.deb"
  DRY_RUN=true
  install_output="$(install_linux)"
  if [[ "$install_output" != *"apt-get install -y --no-install-recommends ${DOWNLOAD_PATH}"* ]]; then
    echo "FAIL: .deb dry-run should install with apt-get so dependencies resolve"
    echo "$install_output"
    exit 1
  fi
)

python3 - "$REPO_ROOT/app/src-tauri/tauri.conf.json" <<'PY'
import json, sys

path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    config = json.load(f)
depends = set(config["bundle"]["linux"]["deb"]["depends"])
required = {
    "libatk-bridge2.0-0",
    "libgbm1",
    "libnspr4",
    "libnss3",
    "libxkbcommon0",
    "libxshmfence1",
}
missing = sorted(required - depends)
if missing:
    print("FAIL: .deb package is missing CEF runtime dependencies: " + ", ".join(missing))
    sys.exit(1)
PY

set +e
missing_channel_output=$(bash "$REPO_ROOT/scripts/install.sh" --channel 2>&1)
missing_channel_rc=$?
set -e
if [[ "$missing_channel_rc" -eq 0 ]]; then
  echo "FAIL: install.sh --channel should fail when the value is missing"
  exit 1
fi
if [[ "$missing_channel_output" != *"Missing value for --channel"* ]]; then
  echo "FAIL: install.sh --channel should explain that the value is missing"
  echo "$missing_channel_output"
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
