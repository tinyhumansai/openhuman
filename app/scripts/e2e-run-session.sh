#!/usr/bin/env bash
#
# Unified WDIO E2E runner — one Appium session, one CEF app launch, all specs.
#
# Architecture:
#   1. Build artefacts must exist (run `pnpm test:e2e:build` first).
#   2. Clean cached app data + write a fresh E2E config.toml pointing at the
#      shared mock backend.
#   3. Launch the built CEF binary directly (cross-platform).
#   4. Wait for the CEF process to expose CDP on 127.0.0.1:19222.
#   5. Start Appium with the `chromium` driver.
#   6. Run wdio against `test/wdio.conf.ts`, which attaches to the running
#      CEF via Appium's Chromium driver. All specs share one session.
#   7. Tear everything down (Appium → CEF → workspace).
#
# Usage:
#   ./app/scripts/e2e-run-session.sh                          # whole suite
#   ./app/scripts/e2e-run-session.sh test/e2e/specs/foo.spec.ts  # single spec
#
set -euo pipefail

SPEC_ARG="${1:-}"
LOG_SUFFIX="${2:-session}"

E2E_MOCK_PORT="${E2E_MOCK_PORT:-18473}"
CEF_CDP_PORT="${CEF_CDP_PORT:-19222}"
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
  if [ -n "$APPIUM_PID" ]; then
    echo "[runner] Stopping Appium (pid $APPIUM_PID)..."
    kill "$APPIUM_PID" 2>/dev/null || true
    wait "$APPIUM_PID" 2>/dev/null || true
  fi
  if [ -n "$APP_PID" ]; then
    echo "[runner] Stopping CEF app (pid $APP_PID)..."
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
    # Tolerate transient races: even after the kill above, a CEF helper
    # may still be flushing CEF/Default/* on a slow Linux runner. The
    # workspace is a per-run mktemp under /tmp; anything left behind is
    # collected by the next CI tmp-cleanup pass. We must not fail the
    # whole job on cleanup leftovers when the test itself passed.
    rm -rf "$CREATED_TEMP_WORKSPACE" 2>/dev/null || true
  fi
  if [ -n "$CREATED_TEMP_CEF_CACHE" ]; then
    rm -rf "$CREATED_TEMP_CEF_CACHE" 2>/dev/null || true
  fi
  if [ -n "$E2E_CONFIG_BACKUP" ] && [ -f "$E2E_CONFIG_BACKUP" ]; then
    mv "$E2E_CONFIG_BACKUP" "$E2E_CONFIG_FILE"
  elif [ -n "$E2E_CONFIG_FILE" ] && [ -f "$E2E_CONFIG_FILE" ]; then
    rm -f "$E2E_CONFIG_FILE"
  fi
}
trap cleanup EXIT

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"
export BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT}"
export APPIUM_PORT
export CEF_CDP_PORT

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
# on every platform.
E2E_CONFIG_DIR="$HOME/.openhuman"
E2E_CONFIG_FILE="$E2E_CONFIG_DIR/config.toml"
mkdir -p "$E2E_CONFIG_DIR"
if [ -f "$E2E_CONFIG_FILE" ]; then
  E2E_CONFIG_BACKUP="$E2E_CONFIG_FILE.e2e-backup.$$"
  cp "$E2E_CONFIG_FILE" "$E2E_CONFIG_BACKUP"
  sed -i.bak '/^api_url[[:space:]]*=/d' "$E2E_CONFIG_FILE" && rm -f "$E2E_CONFIG_FILE.bak"
  EXISTING_CONTENT="$(cat "$E2E_CONFIG_FILE")"
  printf 'api_url = "http://127.0.0.1:%s"\n%s\n' "${E2E_MOCK_PORT}" "$EXISTING_CONTENT" > "$E2E_CONFIG_FILE"
else
  printf 'api_url = "http://127.0.0.1:%s"\n' "${E2E_MOCK_PORT}" > "$E2E_CONFIG_FILE"
fi
echo "[runner] Wrote E2E config.toml with api_url=http://127.0.0.1:${E2E_MOCK_PORT}"

DIST_JS="$(ls dist/assets/index-*.js 2>/dev/null | head -1)"
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
if [ -z "${APP_BIN:-}" ] || [ ! -x "$APP_BIN" ]; then
  echo "ERROR: built OpenHuman binary not found. Run 'pnpm test:e2e:build' first." >&2
  exit 1
fi

# ------------------------------------------------------------------------------
# Ensure a dbus session bus exists on Linux.
#
# The Tauri `single-instance` plugin (used inside OpenHuman) talks to dbus
# via zbus on Linux. If DBUS_SESSION_BUS_ADDRESS is missing or set to
# `disabled:` (which is the openhuman_ci container default), the plugin
# panics during plugin setup:
#   panicked at plugins/single-instance/src/platform_impl/linux.rs:57
#   Result::unwrap() on Err(Address("unsupported transport 'disabled'"))
# So start a real session bus with `dbus-launch` and inherit its
# DBUS_SESSION_BUS_ADDRESS for the rest of the runner.
# ------------------------------------------------------------------------------
if [ "$OS" = "Linux" ]; then
  if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] || \
     printf '%s' "${DBUS_SESSION_BUS_ADDRESS:-}" | grep -q '^disabled'; then
    if command -v dbus-launch >/dev/null 2>&1; then
      DBUS_LAUNCH_OUT="$(dbus-launch --sh-syntax)"
      eval "$DBUS_LAUNCH_OUT"
      echo "[runner] Started dbus session bus: $DBUS_SESSION_BUS_ADDRESS"
    else
      echo "[runner] Warning: dbus-launch not available — single-instance plugin may panic."
    fi
  fi
fi

# ------------------------------------------------------------------------------
# Make CEF runtime libraries discoverable.
#
# macOS bundles the framework into `OpenHuman.app/Contents/Frameworks/` so the
# OS resolves it automatically. On Linux + Windows we build with --no-bundle
# (faster, no .deb / .msi staging needed for tests), which means the bare
# binary has no co-located libcef.so / libcef.dll. CEF lives in the
# vendored-tauri-cli cache that `ensure-tauri-cli.sh` pinned via CEF_PATH:
#   $CEF_PATH/<cef-version>/cef_<os>_<arch>/{libcef.so,libcef.dll,Resources,…}
#
# We find the only versioned directory under CEF_PATH and prepend its CEF
# dist dir to LD_LIBRARY_PATH (Linux) / PATH (Windows). On macOS this is a
# no-op — the .app bundle already self-resolves.
# ------------------------------------------------------------------------------
CEF_PATH="${CEF_PATH:-$HOME/Library/Caches/tauri-cef}"

# Pick exactly one CEF distribution per platform. If two cached versions
# coexist (e.g. after a CEF upgrade) `head -1` silently picks an arbitrary
# one and the binary can load the wrong libcef — Linux/Windows then fail
# only on warmed runners. Error out so the failure is loud.
pick_one_cef_dist() {
  local pattern="$1"
  local matches
  mapfile -t matches < <(find "$CEF_PATH" -mindepth 2 -maxdepth 2 -type d -name "$pattern" 2>/dev/null | sort)
  if [ "${#matches[@]}" -gt 1 ]; then
    echo "ERROR: multiple CEF distributions found under $CEF_PATH matching $pattern:" >&2
    printf '  %s\n' "${matches[@]}" >&2
    echo "Set CEF_PATH to a directory containing exactly one cef_* version." >&2
    return 1
  fi
  printf '%s' "${matches[0]:-}"
}

case "$OS" in
  Linux)
    CEF_DIST_DIR="$(pick_one_cef_dist 'cef_linux_*')" || exit 1
    if [ -n "$CEF_DIST_DIR" ] && [ -d "$CEF_DIST_DIR" ]; then
      export LD_LIBRARY_PATH="$CEF_DIST_DIR:${LD_LIBRARY_PATH:-}"
      echo "[runner] LD_LIBRARY_PATH includes: $CEF_DIST_DIR"
    else
      echo "[runner] Warning: no CEF Linux distribution found under $CEF_PATH — libcef.so may fail to load."
    fi
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    CEF_DIST_DIR="$(pick_one_cef_dist 'cef_windows_*')" || exit 1
    if [ -n "$CEF_DIST_DIR" ] && [ -d "$CEF_DIST_DIR" ]; then
      # Windows uses PATH for DLL resolution. Use the native Windows path form
      # so the CEF binary itself can find libcef.dll even though we're running
      # under git-bash.
      if command -v cygpath >/dev/null 2>&1; then
        WIN_CEF_DIST="$(cygpath -w "$CEF_DIST_DIR")"
      else
        WIN_CEF_DIST="$CEF_DIST_DIR"
      fi
      export PATH="$CEF_DIST_DIR:$PATH"
      echo "[runner] PATH includes: $CEF_DIST_DIR (win: $WIN_CEF_DIST)"
    else
      echo "[runner] Warning: no CEF Windows distribution found under $CEF_PATH — libcef.dll may fail to load."
    fi
    ;;
esac

# ------------------------------------------------------------------------------
# Launch CEF app — CDP on 19222 is already enabled in lib.rs (see CLAUDE.md).
# ------------------------------------------------------------------------------
LOG_DIR="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
APP_LOG="$LOG_DIR/openhuman-e2e-app-${LOG_SUFFIX}.log"
APP_ARGS=()
# CEF/Chromium needs extra coaxing in headless / containerized Linux runs:
#
#   --no-sandbox            crbug.com/638180 — refuses to start as root
#                           without this. The openhuman_ci docker image
#                           runs as uid 0.
#   --disable-dev-shm-usage docker /dev/shm is often 64 MB; Chromium
#                           assumes ≥2 GB and crashes mid-startup
#                           ("Failed global descriptor lookup: 7" in the
#                           zygote helper).
#   --disable-gpu           no GPU in the CI container.
#   --no-zygote             skips the zygote launcher that wants dbus.
#
# Apply only on Linux. macOS/Windows runners are unprivileged users with a
# real display / GPU; leaving the sandbox on there is correct.
case "$OS" in
  Linux)
    if [ "$(id -u 2>/dev/null || echo 0)" = "0" ]; then
      APP_ARGS+=("--no-sandbox")
    fi
    APP_ARGS+=(
      "--disable-dev-shm-usage"
      "--disable-gpu"
      "--no-zygote"
    )
    echo "[runner] Linux CEF args: ${APP_ARGS[*]}"
    ;;
esac
echo "[runner] Launching CEF app: $APP_BIN ${APP_ARGS[*]:-}"
echo "[runner]   App logs: $APP_LOG"
# `${APP_ARGS[@]+"${APP_ARGS[@]}"}` is the idiom for expanding a possibly-
# empty array under `set -u`. On macOS APP_ARGS is empty; on Linux it has
# --no-sandbox etc. Without this guard bash errors with "APP_ARGS[@]:
# unbound variable" and the app never launches.
"$APP_BIN" ${APP_ARGS[@]+"${APP_ARGS[@]}"} > "$APP_LOG" 2>&1 &
APP_PID=$!

echo "[runner] Waiting for CDP at http://127.0.0.1:${CEF_CDP_PORT}/json/version ..."
CDP_VERSION_JSON=""
for i in $(seq 1 60); do
  CDP_VERSION_JSON="$(curl -sf "http://127.0.0.1:${CEF_CDP_PORT}/json/version" 2>/dev/null || true)"
  if [ -n "$CDP_VERSION_JSON" ]; then
    echo "[runner] CDP is ready."
    break
  fi
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    echo "ERROR: CEF app exited before CDP came up. App log follows:" >&2
    echo "----- $APP_LOG -----" >&2
    cat "$APP_LOG" >&2 || true
    echo "----- end log -----" >&2
    exit 1
  fi
  if [ "$i" -eq 60 ]; then
    echo "ERROR: CDP did not come up within 60s. App log follows:" >&2
    echo "----- $APP_LOG -----" >&2
    cat "$APP_LOG" >&2 || true
    echo "----- end log -----" >&2
    exit 1
  fi
  sleep 1
done

# Wait for the main app target (tauri.localhost) to be visible, then close the
# CEF prewarm child-webview slot (about:blank) so chromedriver attaches to the
# real app. Without this, debuggerAddress picks the first page target — which
# is the prewarm — and chromedriver's session is bound to a target CEF may
# garbage-collect mid-test, killing the session with "session terminated".
echo "[runner] Waiting for main app CDP target (tauri.localhost) ..."
MAIN_TARGET_ID=""
for i in $(seq 1 60); do
  TARGETS_JSON="$(curl -sf "http://127.0.0.1:${CEF_CDP_PORT}/json/list" 2>/dev/null || true)"
  MAIN_TARGET_ID="$(printf '%s' "$TARGETS_JSON" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for t in data:
    if t.get("type") == "page" and "tauri.localhost" in (t.get("url") or ""):
        print(t.get("id", ""))
        break
' 2>/dev/null || true)"
  if [ -n "$MAIN_TARGET_ID" ]; then
    echo "[runner] Main app target: $MAIN_TARGET_ID"
    break
  fi
  if [ "$i" -eq 60 ]; then
    echo "[runner] Warning: main app target never appeared — chromedriver may attach to the wrong target."
    break
  fi
  sleep 1
done

if [ -n "$MAIN_TARGET_ID" ]; then
  printf '%s' "$TARGETS_JSON" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for t in data:
    if t.get("type") == "page" and (t.get("url") or "").startswith("about:"):
        print(t.get("id", ""))
' 2>/dev/null | while read -r STALE_ID; do
    if [ -n "$STALE_ID" ]; then
      echo "[runner] Closing prewarm/about:blank target: $STALE_ID"
      curl -sf "http://127.0.0.1:${CEF_CDP_PORT}/json/close/$STALE_ID" > /dev/null 2>&1 || true
    fi
  done
fi

# ------------------------------------------------------------------------------
# Resolve a chromedriver whose major matches CEF's bundled Chromium.
#
# chromedriver is strict about same-major-version matching (e.g. cd148 cannot
# talk to Chrome 146). The Appium chromium driver ships its own bundled
# chromedriver but that often drifts ahead of CEF's pinned Chromium.
# Solution: parse Chromium's version from /json/version, then download the
# matching chromedriver from Chrome for Testing into a workspace-local cache.
# We pass the resulting binary to Appium via the `appium:chromedriverExecutable`
# capability (wired in wdio.conf.ts via E2E_CHROMEDRIVER_PATH).
# ------------------------------------------------------------------------------
CHROMIUM_FULL_VERSION="$(echo "$CDP_VERSION_JSON" | sed -n 's/.*"Browser": *"[^/]*\/\([^"]*\)".*/\1/p' | head -1)"
if [ -z "$CHROMIUM_FULL_VERSION" ]; then
  CHROMIUM_FULL_VERSION="$(echo "$CDP_VERSION_JSON" | sed -n 's/.*"Browser": *"\([^"]*\)".*/\1/p' | sed 's/^[^/]*\///' | head -1)"
fi
echo "[runner] CEF Chromium version: ${CHROMIUM_FULL_VERSION:-<unknown>}"

case "$OS" in
  Darwin)
    ARCH="$(uname -m)"
    case "$ARCH" in
      arm64) CFT_PLATFORM="mac-arm64" ;;
      x86_64) CFT_PLATFORM="mac-x64" ;;
      *) CFT_PLATFORM="mac-arm64" ;;
    esac
    CD_EXE_NAME="chromedriver"
    ;;
  Linux)
    CFT_PLATFORM="linux64"
    CD_EXE_NAME="chromedriver"
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    CFT_PLATFORM="win64"
    CD_EXE_NAME="chromedriver.exe"
    ;;
esac

CD_CACHE_DIR="$APP_DIR/test/e2e/.cache/chromedriver/${CHROMIUM_FULL_VERSION:-unknown}-${CFT_PLATFORM}"
CD_BINARY="$CD_CACHE_DIR/chromedriver-${CFT_PLATFORM}/${CD_EXE_NAME}"

if [ -n "$CHROMIUM_FULL_VERSION" ] && [ ! -x "$CD_BINARY" ]; then
  CD_URL="https://storage.googleapis.com/chrome-for-testing-public/${CHROMIUM_FULL_VERSION}/${CFT_PLATFORM}/chromedriver-${CFT_PLATFORM}.zip"
  echo "[runner] Downloading matching chromedriver: $CD_URL"
  mkdir -p "$CD_CACHE_DIR"
  CD_ZIP="$CD_CACHE_DIR/chromedriver.zip"
  if curl -fSL "$CD_URL" -o "$CD_ZIP"; then
    if command -v unzip >/dev/null 2>&1; then
      (cd "$CD_CACHE_DIR" && unzip -o -q chromedriver.zip)
    else
      # The openhuman_ci docker image doesn't ship `unzip`; use Python's
      # stdlib zipfile so we don't have to add a system package install.
      python3 -c "import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" \
        "$CD_ZIP" "$CD_CACHE_DIR"
    fi
    chmod +x "$CD_BINARY" 2>/dev/null || true
  else
    echo "[runner] Warning: chromedriver $CHROMIUM_FULL_VERSION not on Chrome for Testing; falling back to Appium-bundled chromedriver."
  fi
fi

if [ -x "$CD_BINARY" ]; then
  export E2E_CHROMEDRIVER_PATH="$CD_BINARY"
  echo "[runner] Using chromedriver: $E2E_CHROMEDRIVER_PATH"
fi

# ------------------------------------------------------------------------------
# Start Appium (chromium driver)
# ------------------------------------------------------------------------------
# shellcheck source=/dev/null
source "$SCRIPT_DIR/e2e-resolve-node-appium.sh"

# Make sure the chromium driver is installed. `appium driver list --installed`
# exits non-zero on parse errors in some Appium versions, so just attempt the
# install and ignore "already installed" output.
echo "[runner] Ensuring Appium chromium driver is installed..."
"$APPIUM_BIN" driver install --source=npm appium-chromium-driver >/dev/null 2>&1 || true

APPIUM_LOG="$LOG_DIR/appium-e2e-${LOG_SUFFIX}.log"

# Fail fast if something else is already serving on the Appium port. Otherwise
# `curl /status` succeeds against the stale server while our just-launched
# Appium dies with EADDRINUSE — we'd silently drive the wrong instance.
if curl -sf "http://127.0.0.1:$APPIUM_PORT/status" >/dev/null 2>&1; then
  echo "ERROR: Appium is already listening on port $APPIUM_PORT. Stop the stale server or set APPIUM_PORT." >&2
  exit 1
fi

echo "[runner] Starting Appium on port $APPIUM_PORT"
echo "[runner]   Appium logs: $APPIUM_LOG"
"$APPIUM_BIN" --port "$APPIUM_PORT" --relaxed-security > "$APPIUM_LOG" 2>&1 &
APPIUM_PID=$!

for i in $(seq 1 30); do
  # If Appium crashed between forks (e.g. unhandled driver-load error), bail
  # out instead of polling /status for 30s.
  if ! kill -0 "$APPIUM_PID" 2>/dev/null; then
    echo "ERROR: Appium exited before becoming ready. Appium log follows:" >&2
    echo "----- $APPIUM_LOG -----" >&2
    cat "$APPIUM_LOG" >&2 || true
    echo "----- end log -----" >&2
    exit 1
  fi
  if curl -sf "http://127.0.0.1:$APPIUM_PORT/status" >/dev/null 2>&1; then
    echo "[runner] Appium is ready."
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "ERROR: Appium did not start within 30 seconds." >&2
    exit 1
  fi
  sleep 1
done

# ------------------------------------------------------------------------------
# Run WDIO
# ------------------------------------------------------------------------------
if [ -n "$SPEC_ARG" ]; then
  echo "[runner] Running single spec: $SPEC_ARG"
  pnpm exec wdio run test/wdio.conf.ts --spec "$SPEC_ARG"
else
  echo "[runner] Running full E2E suite (single shared session)..."
  pnpm exec wdio run test/wdio.conf.ts
fi
