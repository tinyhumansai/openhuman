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
resolve_appium_bin() {
  if command -v appium >/dev/null 2>&1; then
    command -v appium
    return 0
  fi

  local npm_prefix=""
  npm_prefix="$(npm prefix -g 2>/dev/null || true)"
  for candidate in \
    "${npm_prefix}/bin/appium" \
    "/usr/bin/appium" \
    "/usr/local/bin/appium"; do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then
      echo "$candidate"
      return 0
    fi
  done

  return 1
}

if ! command -v appium >/dev/null 2>&1; then
  echo "[e2e-bootstrap] Installing Appium 3 + chromium driver..."
  npm install -g appium@3 >/dev/null
fi
APPIUM_BIN="$(resolve_appium_bin)" || {
  echo "[e2e-bootstrap] ERROR: Appium installed but binary not found on PATH" >&2
  exit 1
}
"$APPIUM_BIN" driver install --source=npm appium-chromium-driver >/dev/null

# 3. JS deps — only run pnpm install when node_modules is empty or stale.
#    `pnpm install --frozen-lockfile` is a no-op when up to date, so this
#    is cheap on warm re-runs.
cd /workspace
if [ ! -d node_modules ] || [ ! -e node_modules/.modules.yaml ]; then
  echo "[e2e-bootstrap] Installing JS deps..."
  pnpm install --frozen-lockfile
fi

# 4. Ensure stub env files exist (CI does this too).
#
# Local developers often symlink `.env` to a secrets directory outside the
# repo (e.g. `../secrets/openhuman/.env`). Inside this container only
# `/workspace` is bind-mounted, so that symlink target doesn't exist —
# `[ -f .env ]` returns false on a broken symlink, and `touch .env` then
# fails with "No such file or directory" because touch resolves through
# the dangling link. Detect that case and use a per-container override
# in $HOME (exported as a dotenv path) so we don't clobber the host
# symlink via the bind mount.
ensure_stub_env() {
  local target="$1"
  if [ -e "$target" ] || [ ! -L "$target" ]; then
    [ -e "$target" ] || touch "$target"
    return
  fi
  # Broken symlink → leave it alone, write a sibling stub the build can
  # source via OPENHUMAN_DOTENV_FILE / VITE handles env loading itself.
  local stub
  stub="${HOME}/openhuman-e2e-$(echo "$target" | tr '/' '_').env"
  : > "$stub"
  echo "[e2e-bootstrap] $target is a broken symlink; using stub $stub"
}
ensure_stub_env .env
ensure_stub_env app/.env

echo "[e2e-bootstrap] Ready (DISPLAY=$DISPLAY)."
exec "$@"
