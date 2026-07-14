#!/usr/bin/env bash
#
# Run the openhuman ↔ tiny.place messaging UI e2e (Playwright).
#
# Reuses the app's standard web-session harness (app/scripts/e2e-web-session.sh)
# — mock cloud backend + standalone core + static web host — but points the
# core's *tiny.place* backend at a real one via TINYPLACE_API_BASE_URL, so the
# Messaging screen exchanges real Signal DMs. The peer core is launched by the
# spec itself.
#
# Steps (each skipped if already satisfied):
#   1. Ensure a tiny.place backend is reachable (auto-start one unless MANAGE_STACK=0).
#   2. Ensure the web bundle + core binary are built.
#   3. Run the Playwright spec through the web session.
#
# Env knobs: same as run.sh, plus:
#   HEADED=1   run Playwright headed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPENHUMAN_ROOT="$(cd "$HERE/../.." && pwd)"
UMBRELLA_ROOT="$(cd "$OPENHUMAN_ROOT/.." && pwd)"
APP_DIR="$OPENHUMAN_ROOT/app"

BACKEND_PORT="${BACKEND_PORT:-18080}"
export TINYPLACE_API_BASE_URL="${TINYPLACE_API_BASE_URL:-http://localhost:${BACKEND_PORT}}"
# Reuse the already-built debug core instead of a separate e2e-web target.
export E2E_WEB_CORE_TARGET_DIR="${E2E_WEB_CORE_TARGET_DIR:-$OPENHUMAN_ROOT/target}"
MANAGE_STACK="${MANAGE_STACK:-1}"
COMPOSE_PROJECT="tinyplace-ohe2e"
SPEC="test/playwright/specs/tinyplace-messaging.spec.ts"

log() { printf '\033[1;35m[messaging-ui-e2e]\033[0m %s\n' "$*"; }
backend_up() { curl -fsS "${TINYPLACE_API_BASE_URL%/}/healthz" >/dev/null 2>&1; }

STARTED_STACK=0
cleanup() {
  if [ "$STARTED_STACK" = "1" ]; then
    log "tearing down backend stack ($COMPOSE_PROJECT)"
    ( cd "$UMBRELLA_ROOT" && MONGO_PORT="${MONGO_PORT:-37017}" REDIS_PORT="${REDIS_PORT:-16379}" BACKEND_PORT="$BACKEND_PORT" \
        docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f e2e/docker-compose.e2e.yml down -v >/dev/null 2>&1 || true )
  fi
}
trap cleanup EXIT

# 1) Backend
if backend_up; then
  log "backend reachable at $TINYPLACE_API_BASE_URL"
elif [ "$MANAGE_STACK" != "0" ]; then
  log "starting isolated backend stack (mongo+redis+backend, static verifier) on :$BACKEND_PORT"
  ( cd "$UMBRELLA_ROOT" && MONGO_PORT="${MONGO_PORT:-37017}" REDIS_PORT="${REDIS_PORT:-16379}" BACKEND_PORT="$BACKEND_PORT" \
      docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f e2e/docker-compose.e2e.yml up --build -d mongo redis backend )
  STARTED_STACK=1
  for i in $(seq 1 60); do backend_up && break; sleep 2; done
  backend_up || { log "ERROR: backend never became healthy"; exit 1; }
  log "backend healthy"
else
  log "ERROR: backend not reachable and MANAGE_STACK=0."; exit 1
fi

# 2) Build web bundle + core if needed.
if [ ! -f "$APP_DIR/dist-web/index.html" ] || [ ! -x "$E2E_WEB_CORE_TARGET_DIR/debug/openhuman-core" ]; then
  log "building web e2e bundle (+ core if missing)…"
  ( cd "$OPENHUMAN_ROOT" && bash app/scripts/e2e-web-build.sh )
fi
# Ensure the Playwright browser is present.
( cd "$APP_DIR" && pnpm exec playwright install chromium chromium-headless-shell >/dev/null 2>&1 || true )

# 3) Run the spec through the web session harness.
log "running Playwright spec: $SPEC (tiny.place backend: $TINYPLACE_API_BASE_URL)"
ARGS=("$SPEC")
[ "${HEADED:-}" = "1" ] && ARGS+=("--headed")
exec bash "$APP_DIR/scripts/e2e-web-session.sh" "${ARGS[@]}"
