#!/usr/bin/env bash
#
# Canonical "agent review" run: builds the app if needed, runs the
# agent-review spec, and prints the artifact directory so agents (and
# humans) can inspect screenshots, page-source dumps, and mock request
# logs on disk.
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
  echo "[agent-review] building E2E app with in-process core"
  pnpm --filter openhuman-app test:e2e:build
else
  echo "[agent-review] --skip-build set; reusing existing build"
fi

echo "[agent-review] running spec test/e2e/specs/agent-review.spec.ts"
bash "$APP_DIR/scripts/e2e-run-spec.sh" test/e2e/specs/agent-review.spec.ts agent-review

# Find the most recent run dir for this label.
ARTIFACT_ROOT="${E2E_ARTIFACT_ROOT:-$APP_DIR/test/e2e/artifacts}"
if [ -d "$ARTIFACT_ROOT" ]; then
  LATEST="$(ls -1dt "$ARTIFACT_ROOT"/*"-$LABEL" 2>/dev/null | head -n 1 || true)"
  if [ -n "$LATEST" ]; then
    echo
    echo "[agent-review] ==========================================="
    echo "[agent-review] artifact dir: $LATEST"
    echo "[agent-review] ==========================================="
    ls -1 "$LATEST" 2>/dev/null || true
  else
    echo "[agent-review] no artifact dir found under $ARTIFACT_ROOT"
  fi
else
  echo "[agent-review] artifact root missing: $ARTIFACT_ROOT"
fi
