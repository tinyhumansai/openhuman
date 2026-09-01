#!/usr/bin/env bash
# Browser-hosted variant of `pnpm dev:app`.
#
# `dev:app` renders the UI in the Tauri shell, which since the CEF -> Wry
# migration (#5456) is WKWebView on macOS / WebView2 on Windows / WebKitGTK on
# Linux. None of those speak the Chrome DevTools Protocol, so no CDP client --
# including the chrome-devtools MCP an agent drives -- can attach to the
# desktop window. This script runs the same SPA in a real browser instead:
#
#   openhuman-core (JSON-RPC :7788) <- fetch -- Vite dev server (:1420) in Chrome
#
# The renderer takes the browser path in `coreRpcClient` (`isTauri()` is false),
# reading the endpoint and bearer from `localStorage`. Those are seeded by the
# dev-server-only `/__dev-connect` route (see `devConnectPlugin` in
# `app/vite.config.ts`), which is where the browser is pointed first.
#
# Usage:
#   pnpm dev:app:web                # start core + vite, open the browser
#   pnpm dev:app:web --no-browser   # start both, just print the URL (for agents)
#
# Env:
#   OPENHUMAN_DEV_PORT    Vite port (default 1420)
#   OPENHUMAN_CORE_PORT   core port (default 7788; auto-advances if taken)
#   OPENHUMAN_CORE_TOKEN  bearer to use (default: generated per run)
#   OPENHUMAN_WORKSPACE   core workspace (default: the usual ~/.openhuman)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

open_browser=1
for arg in "$@"; do
  case "$arg" in
    --no-browser) open_browser=0 ;;
    *) echo "[dev:web] unknown argument: $arg" >&2; exit 2 ;;
  esac
done

if [[ -f "$REPO_ROOT/.env" ]]; then
  # shellcheck source=load-dotenv.sh
  source "$SCRIPT_DIR/load-dotenv.sh" "$REPO_ROOT/.env"
fi

port_is_free() {
  ! nc -z 127.0.0.1 "$1" >/dev/null 2>&1
}

validate_port() {
  local raw="${1//[[:space:]]/}" fallback="$2" label="$3"
  if [[ "$raw" =~ ^[0-9]+$ ]] && (( 10#$raw >= 1 && 10#$raw <= 65535 )); then
    echo "$raw"
  else
    echo "[dev:web] WARNING: invalid $label='$raw'; using $fallback" >&2
    echo "$fallback"
  fi
}

dev_port="$(validate_port "${OPENHUMAN_DEV_PORT:-1420}" 1420 OPENHUMAN_DEV_PORT)"
core_port="$(validate_port "${OPENHUMAN_CORE_PORT:-7788}" 7788 OPENHUMAN_CORE_PORT)"

# Vite runs with strictPort, so a busy dev port is a hard stop rather than a
# silent move to a URL the browser is never pointed at.
if ! port_is_free "$dev_port"; then
  echo "[dev:web] ERROR: port $dev_port is in use (another dev server?)." >&2
  echo "[dev:web] Stop it, or set OPENHUMAN_DEV_PORT to a free port." >&2
  exit 1
fi

# The core port may legitimately be taken by another checkout's core. Advance
# rather than fail: this script always talks to the core it started itself, so
# reusing a stranger's would mean debugging against someone else's workspace.
original_core_port="$core_port"
while ! port_is_free "$core_port"; do
  core_port=$(( core_port + 1 ))
  if (( core_port > original_core_port + 20 )); then
    echo "[dev:web] ERROR: no free core port near $original_core_port." >&2
    exit 1
  fi
done
if [[ "$core_port" != "$original_core_port" ]]; then
  echo "[dev:web] port $original_core_port busy; core will use $core_port"
fi

# A blank OPENHUMAN_CORE_TOKEN does NOT disable auth: `init_rpc_token`
# (src/core/auth.rs) trims it, treats empty as unset, and falls through to
# generating a token and writing it to {workspace}/core.token. So an explicit
# value is the only way both sides agree on a bearer without reading that file.
core_token="${OPENHUMAN_CORE_TOKEN:-}"
core_token="${core_token//[[:space:]]/}"
if [[ -z "$core_token" ]]; then
  core_token="$(openssl rand -hex 32)"
fi
export OPENHUMAN_CORE_TOKEN="$core_token"

core_bin="$REPO_ROOT/target/debug/openhuman-core"
# Always run the (incremental) build rather than only when the binary is
# missing — the normal `tauri dev` path does the same. Skipping this once a
# binary exists means a later run silently executes an arbitrarily stale
# core against a current frontend, which is misleading to debug against.
echo "[dev:web] building openhuman-core…"
# GGML_NATIVE=OFF is the documented Apple-Silicon workaround for llama.cpp.
GGML_NATIVE=OFF cargo build --manifest-path "$REPO_ROOT/Cargo.toml" \
  --bin openhuman-core

core_pid=""
vite_pid=""
cleanup() {
  trap - EXIT INT TERM
  [[ -n "$vite_pid" ]] && kill "$vite_pid" 2>/dev/null || true
  [[ -n "$core_pid" ]] && kill "$core_pid" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "[dev:web] starting openhuman-core on :$core_port"
OPENHUMAN_CORE_PORT="$core_port" "$core_bin" serve &
core_pid=$!

for _ in $(seq 1 60); do
  if curl -sf -m 2 "http://127.0.0.1:$core_port/health" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$core_pid" 2>/dev/null; then
    echo "[dev:web] ERROR: core exited during startup." >&2
    exit 1
  fi
  sleep 1
done

if ! curl -sf -m 2 "http://127.0.0.1:$core_port/health" >/dev/null 2>&1; then
  echo "[dev:web] ERROR: core did not become healthy on :$core_port." >&2
  exit 1
fi

# Fail loudly here rather than letting the browser hit an opaque 401 later.
rpc_status=$(curl -s -o /dev/null -w '%{http_code}' -m 10 \
  -X POST "http://127.0.0.1:$core_port/rpc" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $core_token" \
  -d '{"jsonrpc":"2.0","id":1,"method":"openhuman.health_check","params":{}}')
if [[ "$rpc_status" != "200" ]]; then
  echo "[dev:web] ERROR: authenticated RPC probe returned HTTP $rpc_status." >&2
  exit 1
fi
echo "[dev:web] core healthy and accepting the dev bearer"

# Read by `import.meta.env` (Vite merges prefixed vars from process.env) and by
# the /__dev-connect route, which seeds it into localStorage for the browser.
export VITE_OPENHUMAN_CORE_RPC_URL="http://127.0.0.1:$core_port/rpc"
export OPENHUMAN_DEV_PORT="$dev_port"

echo "[dev:web] starting vite on :$dev_port"
(cd "$REPO_ROOT/app" && pnpm dev) &
vite_pid=$!

connect_url="http://localhost:$dev_port/__dev-connect"
for _ in $(seq 1 60); do
  if curl -sf -m 2 -o /dev/null "http://localhost:$dev_port/"; then
    break
  fi
  if ! kill -0 "$vite_pid" 2>/dev/null; then
    echo "[dev:web] ERROR: vite exited during startup." >&2
    exit 1
  fi
  sleep 1
done

echo
echo "[dev:web] ready"
echo "[dev:web]   core : http://127.0.0.1:$core_port/rpc"
echo "[dev:web]   open : $connect_url"
echo

if (( open_browser )); then
  if command -v open >/dev/null 2>&1; then
    open "$connect_url"
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$connect_url"
  else
    echo "[dev:web] no opener found; visit the URL above." >&2
  fi
else
  echo "[dev:web] --no-browser: point your CDP client at the URL above."
fi

wait "$vite_pid"
