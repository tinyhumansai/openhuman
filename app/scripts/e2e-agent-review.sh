#!/usr/bin/env bash
#
# Canonical browser "agent review" run: builds the standalone web app if
# needed, runs the Playwright owner, and prints its artifact directory.
#
# Usage:
#   bash app/scripts/e2e-agent-review.sh [--skip-build] [--label <name>]
#
# Artifacts land in:
#   app/test/e2e/artifacts/<timestamp>-<label>/
# unless E2E_ARTIFACT_DIR is set.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"

SKIP_BUILD=0
LABEL="agent-review"

while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --label) LABEL="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; exit 2 ;;
  esac
done

export E2E_ARTIFACT_LABEL="$LABEL"

cd "$REPO_ROOT"

if [ "$SKIP_BUILD" -eq 0 ]; then
  echo "[agent-review] building standalone E2E web app"
  pnpm --filter openhuman-app test:e2e:web:build
else
  echo "[agent-review] --skip-build set; reusing existing web build"
fi

ARTIFACT_ROOT="${E2E_ARTIFACT_ROOT:-$APP_DIR/test/e2e/artifacts}"
ARTIFACT_DIR="${E2E_ARTIFACT_DIR:-$ARTIFACT_ROOT/$(date -u +%Y%m%dT%H%M%SZ)-$LABEL}"
echo "[agent-review] running browser owner test/playwright/specs/agent-review.spec.ts"
bash "$APP_DIR/scripts/e2e-web-session.sh" \
  test/playwright/specs/agent-review.spec.ts \
  --output "$ARTIFACT_DIR"

echo
echo "[agent-review] ==========================================="
echo "[agent-review] artifact dir: $ARTIFACT_DIR"
echo "[agent-review] ==========================================="
ls -1 "$ARTIFACT_DIR" 2>/dev/null || true
