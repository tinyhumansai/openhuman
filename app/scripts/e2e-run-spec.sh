#!/usr/bin/env bash
#
# Legacy per-spec runner — kept as a thin shim that delegates to the
# unified session runner. The harness no longer has separate driver paths
# per platform (Mac2 vs tauri-driver); everything runs on Appium Chromium
# driver attached to CEF's CDP port. See e2e-run-session.sh.
#
# Usage (unchanged):
#   ./app/scripts/e2e-run-spec.sh test/e2e/specs/login-flow.spec.ts [log-suffix]
#
set -euo pipefail

SPEC="${1:?spec path required}"
LOG_SUFFIX="${2:-$(basename "$SPEC" .spec.ts)}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/e2e-run-session.sh" "$SPEC" "$LOG_SUFFIX"
