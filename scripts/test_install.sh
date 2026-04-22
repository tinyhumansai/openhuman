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

# Also test a missing platform produces exit code 3
if resolve_asset_url "$FIXTURE" "linux" "aarch64" 2>/dev/null; then
  echo "FAIL: expected non-zero exit for missing platform linux-aarch64"
  exit 1
fi

echo "PASS"
