#!/usr/bin/env bash
#
# One-shot wrapper: reproduce the Linux E2E job locally on macOS.
#
# Mirrors `.github/workflows/e2e.yml` `e2e-linux` step-for-step inside
# the same CI container so any fix that lands green here lands green on CI.
#
# Usage:
#   ./e2e/run-local.sh                                # smoke spec
#   ./e2e/run-local.sh mega-flow                      # mega-flow spec
#   ./e2e/run-local.sh smoke mega-flow                # both, in order
#   ./e2e/run-local.sh all                            # full suite
#   ./e2e/run-local.sh shell                          # interactive shell
#   ./e2e/run-local.sh build                          # rebuild CEF bundle only
#
# The first run is slow (pulls the image, installs Appium, builds the
# CEF bundle). Subsequent runs are fast — volumes cache everything.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

COMPOSE=(docker compose -f e2e/docker-compose.yml)
RUN=("${COMPOSE[@]}" run --rm e2e)

ensure_built() {
  if [ ! -d "$REPO_ROOT/app/src-tauri/target/debug/bundle" ]; then
    echo "[run-local] CEF bundle not built yet — building (slow first run)..."
    "${RUN[@]}" bash -lc "pnpm --filter openhuman-app test:e2e:build"
  fi
}

run_spec() {
  local name="$1"
  local spec
  case "$name" in
    smoke)      spec="test/e2e/specs/smoke.spec.ts" ;;
    mega-flow)  spec="test/e2e/specs/mega-flow.spec.ts" ;;
    *)          spec="test/e2e/specs/${name}.spec.ts" ;;
  esac
  echo "[run-local] === $name ($spec) ==="
  "${RUN[@]}" bash -lc "bash app/scripts/e2e-run-session.sh '$spec' '$name'"
}

case "${1:-smoke}" in
  shell)
    "${RUN[@]}" bash -l
    ;;
  build)
    "${RUN[@]}" bash -lc "pnpm --filter openhuman-app test:e2e:build"
    ;;
  all)
    ensure_built
    "${RUN[@]}" bash -lc "bash app/scripts/e2e-run-session.sh"
    ;;
  *)
    ensure_built
    # `case "${1:-smoke}"` only defaults the test for matching — the loop
    # below iterates the real `"$@"`, which is empty on a no-arg invocation.
    # Re-set the positional params so a bare `./e2e/run-local.sh` actually
    # runs the smoke spec instead of silently doing nothing.
    if [ "$#" -eq 0 ]; then
      set -- smoke
    fi
    for name in "$@"; do
      run_spec "$name"
    done
    ;;
esac
