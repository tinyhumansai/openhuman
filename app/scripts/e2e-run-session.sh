#!/usr/bin/env bash
#
# WDIO E2E runner — one tauri-driver session, all specs.
#
# Architecture:
#   1. Build artefacts must exist (run `pnpm test:e2e:build` first).
#   2. Clean cached app data + write a fresh E2E config.toml pointing at the
#      shared mock backend.
#   3. Start tauri-driver and wait for its /status endpoint.
#   4. Run wdio against `test/wdio.conf.ts`, which drives the app's native
#      Wry/WebKit webview. All specs share one session.
#   5. Tear everything down (driver -> app -> workspace).
#
# The Appium Chromium-driver backend was removed in #5478: it attached over
# CEF's remote-debugging port, and CDP does not exist under the Wry runtime.
#
# Usage:
#   ./app/scripts/e2e-run-session.sh                          # whole suite
#   ./app/scripts/e2e-run-session.sh test/e2e/specs/foo.spec.ts  # single spec
#
set -euo pipefail

# Accept either:
#   - Zero args             → run the entire `specs` glob from wdio.conf.ts
#   - One spec path arg     → legacy single-spec mode (e2e-run-spec.sh shim)
#   - One spec + log suffix → legacy two-arg mode used by debug runner / CI
#   - N>1 spec paths        → multi-spec mode, one shared session
#
# To disambiguate "spec + suffix" from "two specs", we treat arg2 as a log
# suffix only when it does NOT look like a spec path (i.e. doesn't end in
# `.spec.ts` and doesn't start with `test/`).
SPEC_ARGS=()
LOG_SUFFIX="session"
if [ "$#" -ge 1 ]; then
  SPEC_ARGS+=("$1")
  if [ "$#" -eq 2 ] && [[ "$2" != *.spec.ts && "$2" != test/* ]]; then
    LOG_SUFFIX="$2"
  else
    shift
    while [ "$#" -gt 0 ]; do
      SPEC_ARGS+=("$1")
      shift
    done
  fi
fi
# Back-compat: SPEC_ARG is the first spec (only used in stale log lines below).
SPEC_ARG="${SPEC_ARGS[0]:-}"

E2E_MOCK_PORT="${E2E_MOCK_PORT:-18473}"
APPIUM_PORT="${APPIUM_PORT:-4723}"
OS="$(uname)"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR"

CREATED_TEMP_WORKSPACE=""
APPIUM_PID=""
APP_PID=""
E2E_CONFIG_BACKUP=""
E2E_CONFIG_FILE=""
CREATED_TEMP_CEF_CACHE=""

# ------------------------------------------------------------------------------
# Workspace + config
# ------------------------------------------------------------------------------
if [ -z "${OPENHUMAN_WORKSPACE:-}" ]; then
  OPENHUMAN_WORKSPACE="$(mktemp -d)"
  CREATED_TEMP_WORKSPACE="$OPENHUMAN_WORKSPACE"
  export OPENHUMAN_WORKSPACE
  echo "[runner] Using temporary OPENHUMAN_WORKSPACE: $OPENHUMAN_WORKSPACE"
else
  echo "[runner] Using OPENHUMAN_WORKSPACE from environment: $OPENHUMAN_WORKSPACE"
fi

# Headless Linux CI does not always have a usable Secret Service/keychain.
# Keep E2E credentials under OPENHUMAN_WORKSPACE so auth state is deterministic
# and gets cleaned up with the rest of the test workspace.
: "${OPENHUMAN_KEYRING_BACKEND:=file}"
export OPENHUMAN_KEYRING_BACKEND
echo "[runner] Using OPENHUMAN_KEYRING_BACKEND: $OPENHUMAN_KEYRING_BACKEND"

# Place the CEF cache directory OUTSIDE the workspace. By default the Tauri
# shell roots it under `$OPENHUMAN_WORKSPACE/users/<id>/cef`, but our
# `mega-flow` spec calls `openhuman.config_reset_local_data` between
# sub-scenarios — that RPC does `remove_dir_all($OPENHUMAN_WORKSPACE)`,
# which yanks CEF's cache out from under the running process and kills
# the WebDriver session (every later sub-test then fails with
# "invalid session id"). Pointing CEF at a sibling tmpdir via the
# `OPENHUMAN_CEF_CACHE_PATH` escape hatch (`cef_profile.rs:7`) keeps it
# unaffected by the reset.
if [ -z "${OPENHUMAN_CEF_CACHE_PATH:-}" ]; then
  OPENHUMAN_CEF_CACHE_PATH="$(mktemp -d)"
  CREATED_TEMP_CEF_CACHE="$OPENHUMAN_CEF_CACHE_PATH"
  export OPENHUMAN_CEF_CACHE_PATH
  echo "[runner] Using temporary OPENHUMAN_CEF_CACHE_PATH: $OPENHUMAN_CEF_CACHE_PATH"
fi

if [ "${OPENHUMAN_SERVICE_MOCK:-0}" = "1" ] && [ -z "${OPENHUMAN_SERVICE_MOCK_STATE_FILE:-}" ]; then
  OPENHUMAN_SERVICE_MOCK_STATE_FILE="$OPENHUMAN_WORKSPACE/service-mock-state.json"
  export OPENHUMAN_SERVICE_MOCK_STATE_FILE
fi

cleanup() {
  local status=$?
  set +e
  if [ -n "$APPIUM_PID" ]; then
    echo "[runner] Stopping driver (pid $APPIUM_PID)..."
    # tauri-driver launches WebKitWebDriver as a child. Killing only the
    # tauri-driver parent leaves that native child holding the WebDriver port;
    # the next per-spec runner then observes its stale /status response and
    # hangs when it tries to create a session. Snapshot descendants before the
    # parent is reaped, just as we do for the app process below.
    DRIVER_CHILD_PIDS="$(pgrep -P "$APPIUM_PID" 2>/dev/null || true)"
    pkill -TERM -P "$APPIUM_PID" 2>/dev/null || true
    kill "$APPIUM_PID" 2>/dev/null || true
    wait "$APPIUM_PID" 2>/dev/null || true
    sleep 1
    if [ -n "$DRIVER_CHILD_PIDS" ]; then
      for pid in $DRIVER_CHILD_PIDS; do
        kill -KILL "$pid" 2>/dev/null || true
      done
    fi
  fi
  if [ -n "$APP_PID" ]; then
    echo "[runner] Stopping app (pid $APP_PID)..."
    # CEF spawns helper child processes (zygote, GPU, renderers) that
    # the parent does not reap on SIGTERM. If we only `kill $APP_PID`
    # the parent exits but children keep writing into the temp
    # workspace, and the `rm -rf` below races them and fails with
    # "Directory not empty" on Linux runners — even though the WDIO
    # spec itself passed. Reap the whole process tree before cleanup.
    #
    # CRITICAL: capture child PIDs **before** killing the parent.
    # The instant the parent exits, the kernel reparents its children
    # to init (PID 1). After that, `pkill -P "$APP_PID"` matches
    # nothing because no process has the dying parent as its PPID
    # anymore. Snapshot the PIDs while the relationship still exists,
    # then signal them directly by PID.
    CHILD_PIDS="$(pgrep -P "$APP_PID" 2>/dev/null || true)"
    pkill -TERM -P "$APP_PID" 2>/dev/null || true
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
    # Brief grace period so CEF helpers can flush their CEF/Default
    # files and exit on the SIGTERM we already sent. Anything that
    # ignored it gets SIGKILLed by the captured-PID sweep below.
    sleep 1
    if [ -n "$CHILD_PIDS" ]; then
      for pid in $CHILD_PIDS; do
        kill -KILL "$pid" 2>/dev/null || true
      done
    fi
  fi
  if [ -n "$CREATED_TEMP_WORKSPACE" ]; then
    for attempt in 1 2 3; do
      rm -rf "$CREATED_TEMP_WORKSPACE" 2>/dev/null && break
      echo "[runner] Warning: temporary workspace cleanup failed (attempt $attempt): $CREATED_TEMP_WORKSPACE" >&2
      sleep "$attempt"
    done
    if [ -e "$CREATED_TEMP_WORKSPACE" ]; then
      echo "[runner] Warning: leaving temporary workspace after cleanup retries: $CREATED_TEMP_WORKSPACE" >&2
    fi
  fi
  if [ -n "$CREATED_TEMP_CEF_CACHE" ]; then
    rm -rf "$CREATED_TEMP_CEF_CACHE" 2>/dev/null || true
  fi
  if [ -n "$E2E_CONFIG_BACKUP" ] && [ -f "$E2E_CONFIG_BACKUP" ]; then
    mv "$E2E_CONFIG_BACKUP" "$E2E_CONFIG_FILE" \
      || echo "[runner] Warning: failed to restore E2E config backup: $E2E_CONFIG_BACKUP" >&2
  elif [ -n "$E2E_CONFIG_FILE" ] && [ -f "$E2E_CONFIG_FILE" ]; then
    rm -f "$E2E_CONFIG_FILE" \
      || echo "[runner] Warning: failed to remove generated E2E config: $E2E_CONFIG_FILE" >&2
  fi
  return "$status"
}
trap cleanup EXIT

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"
export BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"
export OPENHUMAN_E2E_MODE="1"
export APPIUM_PORT
# Redirect Telegram Bot API calls to the mock server during E2E runs.
# The mock server (WS-A) serves /bot<token>/* routes on the same port as the
# rest of the mock backend.  The core reads this at TelegramChannel::new() time,
# which runs after the config is fully loaded.
export OPENHUMAN_TELEGRAM_BOT_API_BASE="http://127.0.0.1:${E2E_MOCK_PORT}"
export OPENHUMAN_COMPOSIO_DIRECT_BASE_V2="http://127.0.0.1:${E2E_MOCK_PORT}"
export OPENHUMAN_COMPOSIO_DIRECT_BASE_V3="http://127.0.0.1:${E2E_MOCK_PORT}"

echo "[runner] Killing any running OpenHuman instances..."
case "$OS" in
  Darwin) pkill -f "OpenHuman" 2>/dev/null || true ;;
  Linux)  pkill -f "OpenHuman" 2>/dev/null || true ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    taskkill //F //IM "OpenHuman.exe" 2>/dev/null || true
    ;;
esac
sleep 1

echo "[runner] Cleaning cached app data..."
case "$OS" in
  Darwin)
    rm -rf ~/Library/WebKit/com.openhuman.app
    rm -rf ~/Library/Caches/com.openhuman.app
    rm -rf "$HOME/Library/Application Support/com.openhuman.app"
    rm -rf "$HOME/Library/Saved Application State/com.openhuman.app.savedState"
    ;;
  Linux)
    rm -rf "$HOME/.local/share/com.openhuman.app" 2>/dev/null || true
    rm -rf "$HOME/.cache/com.openhuman.app" 2>/dev/null || true
    rm -rf "$HOME/.config/com.openhuman.app" 2>/dev/null || true
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    rm -rf "${APPDATA:-$HOME/AppData/Roaming}/com.openhuman.app" 2>/dev/null || true
    rm -rf "${LOCALAPPDATA:-$HOME/AppData/Local}/com.openhuman.app" 2>/dev/null || true
    ;;
esac

# Mock URL must reach the core sidecar — XCUITest doesn't inherit env,
# and CEF child processes won't either. Pinning via config.toml works
# on every platform. The runner always sets OPENHUMAN_WORKSPACE above;
# Config::load_or_init gives that path precedence over $HOME/.openhuman.
E2E_CONFIG_DIR="${OPENHUMAN_WORKSPACE:-$HOME/.openhuman}"
E2E_CONFIG_FILE="$E2E_CONFIG_DIR/config.toml"
mkdir -p "$E2E_CONFIG_DIR"
if [ -f "$E2E_CONFIG_FILE" ]; then
  E2E_CONFIG_BACKUP="$E2E_CONFIG_FILE.e2e-backup.$$"
  cp "$E2E_CONFIG_FILE" "$E2E_CONFIG_BACKUP"
fi

# Write a complete E2E config that routes ALL LLM inference through the mock
# server via OpenAiCompatibleProvider (supports_streaming=true).
#
# WHY pre-populate cloud_providers here:
#   The unify_ai_provider_settings migration runs on first startup. If
#   cloud_providers is empty it seeds an OpenHuman entry and sets primary_cloud
#   to that entry — which routes all inference to OpenHumanBackendProvider
#   (supports_streaming=false, always returns non-streaming responses, so the
#   mock server never receives /openai/v1/chat/completions).
#
#   By pre-populating [[cloud_providers]] with a "none" auth mock entry and
#   setting primary_cloud to its id, the migration sees !is_empty() and skips
#   seeding entirely. provider_for_role() resolves unset workloads via
#   primary_cloud → slug "e2e" (non-openhuman) → returns "e2e:" →
#   make_cloud_provider_by_slug → auth_style=none → OpenAiCompatibleProvider
#   → supports_streaming=true → streams to mock at /openai/v1/chat/completions.
cat > "$E2E_CONFIG_FILE" << TOMLEOF
api_url = "http://127.0.0.1:${E2E_MOCK_PORT}"
primary_cloud = "p_e2e_mock"
default_model = "e2e-mock-model"
chat_provider = "e2e:e2e-mock-model"
reasoning_provider = "e2e:e2e-mock-model"
agentic_provider = "e2e:e2e-mock-model"
coding_provider = "e2e:e2e-mock-model"

[[cloud_providers]]
id = "p_e2e_mock"
slug = "e2e"
label = "E2E Mock"
endpoint = "http://127.0.0.1:${E2E_MOCK_PORT}/openai/v1"
auth_style = "none"
default_model = "e2e-mock-model"
TOMLEOF
echo "[runner] Wrote E2E config.toml routing inference to mock at http://127.0.0.1:${E2E_MOCK_PORT}"

DIST_JS="$(find dist/assets -maxdepth 1 -name 'index-*.js' -print -quit 2>/dev/null || true)"
if [ -z "$DIST_JS" ]; then
  echo "ERROR: No frontend bundle found at dist/assets/index-*.js." >&2
  echo "       Run 'pnpm test:e2e:build' first." >&2
  exit 1
fi
if ! grep -q "127.0.0.1:${E2E_MOCK_PORT}" "$DIST_JS"; then
  echo "ERROR: frontend bundle does NOT contain mock server URL (127.0.0.1:${E2E_MOCK_PORT})." >&2
  echo "       Run 'pnpm test:e2e:build' to rebuild." >&2
  exit 1
fi

# ------------------------------------------------------------------------------
# Resolve the built CEF binary for this platform
# ------------------------------------------------------------------------------
resolve_app_binary() {
  case "$OS" in
    Darwin)
      for base in \
        "$APP_DIR/src-tauri/target/debug/bundle/macos/OpenHuman.app/Contents/MacOS/OpenHuman" \
        "$REPO_ROOT/target/debug/bundle/macos/OpenHuman.app/Contents/MacOS/OpenHuman"; do
        if [ -x "$base" ]; then echo "$base"; return; fi
      done
      ;;
    Linux)
      for candidate in \
        "$APP_DIR/src-tauri/target/debug/OpenHuman" \
        "$REPO_ROOT/target/debug/OpenHuman"; do
        if [ -x "$candidate" ]; then echo "$candidate"; return; fi
      done
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      for candidate in \
        "$APP_DIR/src-tauri/target/debug/OpenHuman.exe" \
        "$REPO_ROOT/target/debug/OpenHuman.exe"; do
        if [ -x "$candidate" ]; then echo "$candidate"; return; fi
      done
      ;;
  esac
}

APP_BIN="$(resolve_app_binary)"

# Linux builds use Tauri's Wry/WebKit runtime, not CEF. Drive that native
# webview through tauri-driver instead of waiting for a Chromium CDP endpoint.
if [ "$OS" = "Linux" ] && [ "${E2E_USE_TAURI_DRIVER:-0}" = "1" ]; then
  TAURI_DRIVER_PORT="${TAURI_DRIVER_PORT:-4444}"
  TAURI_DRIVER_LOG="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/tauri-driver-${LOG_SUFFIX}.log"
  echo "[runner] Starting tauri-driver on port $TAURI_DRIVER_PORT"
  tauri-driver --port "$TAURI_DRIVER_PORT" --native-driver "${WEBKIT_WEBDRIVER:-/usr/bin/WebKitWebDriver}" \
    > "$TAURI_DRIVER_LOG" 2>&1 &
  APP_PID=$!
  export TAURI_DRIVER_PORT
  for i in $(seq 1 30); do
    if curl -sf "http://127.0.0.1:$TAURI_DRIVER_PORT/status" >/dev/null 2>&1; then
      break
    fi
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      cat "$TAURI_DRIVER_LOG" >&2 || true
      exit 1
    fi
    sleep 1
  done
  if [ "${#SPEC_ARGS[@]}" -gt 0 ]; then
    WDIO_SPEC_ARGS=()
    for s in "${SPEC_ARGS[@]}"; do WDIO_SPEC_ARGS+=(--spec "$s"); done
    pnpm exec wdio run test/wdio.conf.ts --maxInstances 1 "${WDIO_SPEC_ARGS[@]}"
  else
    pnpm exec wdio run test/wdio.conf.ts --maxInstances 1
  fi
  exit $?
fi
if [ -z "${APP_BIN:-}" ] || [ ! -x "$APP_BIN" ]; then
  echo "ERROR: built OpenHuman binary not found. Run 'pnpm test:e2e:build' first." >&2
  exit 1
fi

# The only supported automation backend is tauri-driver, above. The Appium
# Chromium-driver path that used to live here attached to the app over CEF's
# remote-debugging port; CDP does not exist under the Wry runtime, so it was
# removed in #5478. Reaching this point means the caller is on a platform
# without a driver rather than a misconfiguration, so say so plainly.
echo "ERROR: desktop E2E requires tauri-driver (Linux)." >&2
echo "       Set E2E_USE_TAURI_DRIVER=1 on Linux. macOS/Windows have no" >&2
echo "       supported driver since the CDP harness was removed (#5478)." >&2
exit 1
