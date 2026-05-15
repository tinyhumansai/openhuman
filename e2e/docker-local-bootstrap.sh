#!/usr/bin/env bash
#
# Local-only bootstrap for the Linux E2E Docker container.
#
# The CI image (`ghcr.io/tinyhumansai/openhuman_ci:latest`) intentionally
# does NOT bake in Appium or pnpm deps so the image stays small. CI
# installs them per-job. This wrapper performs the same install steps the
# CI workflow runs (`.github/workflows/e2e.yml` → `e2e-linux`), but only
# when they're missing — so warm re-runs are instant.
#
# Sequence:
#   1. Hand off to the shipped entrypoint (Xvfb + dbus).
#   2. Install Appium 3 + appium-chromium-driver if missing.
#   3. Install JS deps (`pnpm install --frozen-lockfile`) if missing.
#   4. exec the user's command.
#
# Triggered by `e2e/docker-compose.yml` as the container entrypoint
# wrapper. The image's own /docker-entrypoint.sh is invoked from here
# so Xvfb/dbus are still set up.
#
set -euo pipefail

# 1. Run the image's entrypoint logic (Xvfb + dbus) in-process via `source`
#    so the resulting env vars (DISPLAY, DBUS_SESSION_BUS_ADDRESS) reach
#    our exec below. The shipped script ends with `exec "$@"` so we use a
#    trimmed inline version here.
export DISPLAY="${DISPLAY:-:99}"
if ! pgrep -x Xvfb >/dev/null 2>&1; then
  Xvfb "$DISPLAY" -screen 0 1280x1024x24 &
  XVFB_PID=$!
  for _ in 1 2 3 4 5; do
    sleep 0.3
    kill -0 "$XVFB_PID" 2>/dev/null && break
  done
  if ! kill -0 "$XVFB_PID" 2>/dev/null; then
    echo "[e2e-bootstrap] ERROR: Xvfb failed to start" >&2
    exit 1
  fi
fi
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  eval "$(dbus-launch --sh-syntax)"
fi
export RUST_BACKTRACE=1
mkdir -p "${HOME}/.local/share/applications"

# 2. Appium + chromium driver — install once, cache via the npm_global volume.
if ! command -v appium >/dev/null 2>&1; then
  echo "[e2e-bootstrap] Installing Appium 3 + chromium driver..."
  npm install -g appium@3 >/dev/null
  appium driver install --source=npm appium-chromium-driver >/dev/null
fi

# 3. JS deps — only run pnpm install when node_modules is empty or stale.
#    `pnpm install --frozen-lockfile` is a no-op when up to date, so this
#    is cheap on warm re-runs.
cd /workspace
if [ ! -d node_modules ] || [ ! -e node_modules/.modules.yaml ]; then
  echo "[e2e-bootstrap] Installing JS deps..."
  pnpm install --frozen-lockfile
fi

# 4. Ensure stub env files exist (CI does this too).
[ -f .env ] || touch .env
[ -f app/.env ] || touch app/.env

echo "[e2e-bootstrap] Ready (DISPLAY=$DISPLAY)."
exec "$@"
