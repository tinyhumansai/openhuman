#!/usr/bin/env bash
#
# e2e-preflight.sh — Pre-flight environment validation for the E2E test suite.
#
# Checks:
#   1. The E2E app binary/bundle exists for the current platform.
#   2. Node.js and pnpm are available.
#   3. Appium is installed (and the chromium driver is registered).
#   4. Ports 19222, 4723, and 18473 are not blocked by stale processes.
#
# Exits 0 if all hard requirements are met.
# Exits 1 if any hard requirement is missing.
# Warnings are printed for soft issues (occupied ports, missing chromium driver)
# but do not fail the script.
#
set -uo pipefail

# ---------------------------------------------------------------------------
# Color helpers — only when stdout is a terminal.
# ---------------------------------------------------------------------------
if [ -t 1 ]; then
  RED='\033[0;31m'
  YELLOW='\033[1;33m'
  GREEN='\033[0;32m'
  BOLD='\033[1m'
  RESET='\033[0m'
else
  RED='' YELLOW='' GREEN='' BOLD='' RESET=''
fi

info()  { printf "%b[preflight]%b %s\n"     "$BOLD"   "$RESET" "$*"; }
ok()    { printf "%b[preflight] ✓%b %s\n"  "$GREEN"  "$RESET" "$*"; }
warn()  { printf "%b[preflight] ⚠%b  %s\n" "$YELLOW" "$RESET" "$*" >&2; }
fail()  { printf "%b[preflight] ✗%b %s\n"  "$RED"    "$RESET" "$*" >&2; }

ERRORS=0
_fail() { fail "$*"; (( ERRORS++ )) || true; }

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

info "Starting E2E pre-flight checks..."
echo ""

# ---------------------------------------------------------------------------
# 1. App binary / bundle
# ---------------------------------------------------------------------------
info "Checking E2E app bundle..."

PLATFORM="$(uname -s)"
BINARY_FOUND=0
BINARY_PATH=""

case "$PLATFORM" in
  Darwin)
    MACOS_BUNDLE="$APP_DIR/src-tauri/target/debug/bundle/macos/OpenHuman.app"
    if [[ -d "$MACOS_BUNDLE" ]]; then
      BINARY_FOUND=1
      BINARY_PATH="$MACOS_BUNDLE"
    fi
    ;;
  Linux)
    LINUX_BIN="$APP_DIR/src-tauri/target/debug/openhuman"
    LINUX_DEB="$APP_DIR/src-tauri/target/debug/bundle/deb"
    if [[ -f "$LINUX_BIN" ]]; then
      BINARY_FOUND=1
      BINARY_PATH="$LINUX_BIN"
    elif [[ -d "$LINUX_DEB" ]]; then
      BINARY_FOUND=1
      BINARY_PATH="$LINUX_DEB"
    fi
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows*)
    WIN_BIN="$APP_DIR/src-tauri/target/debug/openhuman.exe"
    if [[ -f "$WIN_BIN" ]]; then
      BINARY_FOUND=1
      BINARY_PATH="$WIN_BIN"
    fi
    ;;
  *)
    warn "Unknown platform '$PLATFORM' — cannot verify app bundle path."
    BINARY_FOUND=1  # don't block on unknown platforms
    ;;
esac

if [[ $BINARY_FOUND -eq 1 ]]; then
  ok "App bundle found: $BINARY_PATH"
else
  _fail "E2E build not found for $PLATFORM."
  case "$PLATFORM" in
    Darwin)
      fail "  Expected: $MACOS_BUNDLE"
      ;;
    Linux)
      fail "  Expected: $LINUX_BIN"
      ;;
    MINGW*|MSYS*|CYGWIN*)
      fail "  Expected: $WIN_BIN"
      ;;
  esac
  fail "  Run: pnpm --filter openhuman-app test:e2e:build"
fi

echo ""

# ---------------------------------------------------------------------------
# 2. Node.js + pnpm
# ---------------------------------------------------------------------------
info "Checking Node.js and pnpm..."

if command -v node >/dev/null 2>&1; then
  NODE_VERSION="$(node --version 2>/dev/null || echo 'unknown')"
  ok "node found: $NODE_VERSION"
else
  _fail "node not found. Node.js is required to run WDIO."
fi

if command -v pnpm >/dev/null 2>&1; then
  PNPM_VERSION="$(pnpm --version 2>/dev/null || echo 'unknown')"
  ok "pnpm found: $PNPM_VERSION"
else
  _fail "pnpm not found. Install via: npm install -g pnpm"
fi

echo ""

# ---------------------------------------------------------------------------
# 3. Appium + chromium driver
# ---------------------------------------------------------------------------
info "Checking Appium..."

if command -v appium >/dev/null 2>&1; then
  APPIUM_VERSION="$(appium --version 2>/dev/null || echo 'unknown')"
  ok "appium found: $APPIUM_VERSION"

  # Check for the chromium driver — warn only (e2e-run-session.sh handles this)
  CHROMIUM_INSTALLED=0
  if appium driver list --installed 2>&1 | grep -qi "chromium"; then
    CHROMIUM_INSTALLED=1
    ok "Appium chromium driver is installed"
  fi
  if [[ $CHROMIUM_INSTALLED -eq 0 ]]; then
    warn "Appium chromium driver not found in 'appium driver list --installed'."
    warn "  To install: appium driver install --source=npm appium-chromium-driver"
    warn "  (e2e-run-session.sh will attempt idempotent install at runtime.)"
  fi
else
  _fail "Appium not found."
  fail "  Install: npm install -g appium@3"
  fail "  Then:    appium driver install --source=npm appium-chromium-driver"
fi

echo ""

# ---------------------------------------------------------------------------
# 4. Port availability (warnings only — stale processes are soft blockers)
# ---------------------------------------------------------------------------
info "Checking port availability..."

_check_port() {
  local port="$1"
  local label="$2"
  local pid=""
  # Try lsof first (macOS/Linux), fall back to ss (Linux only)
  if command -v lsof >/dev/null 2>&1; then
    pid=$(lsof -ti tcp:"$port" 2>/dev/null | head -1 || true)
  elif command -v ss >/dev/null 2>&1; then
    pid=$(ss -tlnp "sport = :$port" 2>/dev/null | awk 'NR>1 {match($NF,/pid=([0-9]+)/,a); print a[1]}' | head -1 || true)
  fi

  if [[ -n "$pid" ]]; then
    warn "Port $port ($label) is occupied by PID $pid."
    warn "  If this is a stale process from a prior run, kill it:"
    warn "    kill $pid"
  else
    ok "Port $port ($label) is free"
  fi
}

_check_port 19222 "CEF CDP"
_check_port 4723  "Appium"
_check_port 18473 "mock backend (can be pre-running — OK if deliberate)"

echo ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
if [[ $ERRORS -gt 0 ]]; then
  printf "%b[preflight] PRE-FLIGHT FAILED%b — %d error(s) above must be resolved before running E2E tests.\n" \
    "$RED" "$RESET" "$ERRORS" >&2
  exit 1
fi

printf "%b[preflight] Pre-flight passed%b — environment looks good.\n" "$GREEN" "$RESET"
exit 0
