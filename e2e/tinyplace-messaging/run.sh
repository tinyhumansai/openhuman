#!/usr/bin/env bash
#
# Run the openhuman ↔ tiny.place messaging core e2e.
#
# Steps (each is skipped if already satisfied):
#   1. Ensure a tiny.place backend is reachable at $TINYPLACE_API_BASE_URL.
#      If not, and MANAGE_STACK != 0, bring one up from the umbrella
#      docker-compose (mongo + redis + backend, static payment verifier) on an
#      isolated compose project + ports, and tear it down on exit.
#   2. Ensure the openhuman-core binary is built.
#   3. Run the node:test suite.
#
# Env knobs:
#   TINYPLACE_API_BASE_URL   backend base URL          (default http://localhost:18080)
#   OPENHUMAN_CORE_BIN       path to openhuman-core    (default target/debug/openhuman-core)
#   MANAGE_STACK             1 = auto-manage backend   (default 1)
#   BACKEND_PORT             host port for managed backend (default 18080)
#   VERBOSE                  1 = stream core logs
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPENHUMAN_ROOT="$(cd "$HERE/../.." && pwd)"
UMBRELLA_ROOT="$(cd "$OPENHUMAN_ROOT/.." && pwd)"

BACKEND_PORT="${BACKEND_PORT:-18080}"
export TINYPLACE_API_BASE_URL="${TINYPLACE_API_BASE_URL:-http://localhost:${BACKEND_PORT}}"
export OPENHUMAN_CORE_BIN="${OPENHUMAN_CORE_BIN:-$OPENHUMAN_ROOT/target/debug/openhuman-core}"
MANAGE_STACK="${MANAGE_STACK:-1}"
COMPOSE_PROJECT="tinyplace-ohe2e"

log() { printf '\033[1;36m[messaging-e2e]\033[0m %s\n' "$*"; }

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
  if [ ! -f "$UMBRELLA_ROOT/docker-compose.yml" ] || [ ! -f "$UMBRELLA_ROOT/e2e/docker-compose.e2e.yml" ]; then
    log "ERROR: cannot find umbrella docker-compose to auto-start the backend."
    log "Start a tiny.place backend yourself and set TINYPLACE_API_BASE_URL."
    exit 1
  fi
  log "starting isolated backend stack (mongo+redis+backend, static verifier) on :$BACKEND_PORT"
  ( cd "$UMBRELLA_ROOT" && MONGO_PORT="${MONGO_PORT:-37017}" REDIS_PORT="${REDIS_PORT:-16379}" BACKEND_PORT="$BACKEND_PORT" \
      docker compose -p "$COMPOSE_PROJECT" -f docker-compose.yml -f e2e/docker-compose.e2e.yml up --build -d mongo redis backend )
  STARTED_STACK=1
  log "waiting for backend health…"
  for i in $(seq 1 60); do backend_up && break; sleep 2; done
  backend_up || { log "ERROR: backend never became healthy"; exit 1; }
  log "backend healthy"
else
  log "ERROR: backend not reachable and MANAGE_STACK=0."
  exit 1
fi

# 2) Core binary
if [ ! -x "$OPENHUMAN_CORE_BIN" ]; then
  log "building openhuman-core (this can take a while the first time)…"
  ( cd "$OPENHUMAN_ROOT" && GGML_NATIVE=OFF cargo build --bin openhuman-core --manifest-path Cargo.toml )
fi
log "using core binary: $OPENHUMAN_CORE_BIN"

# 3) Test
log "running node:test suite…"
cd "$HERE"
exec node --test messaging.e2e.mjs
