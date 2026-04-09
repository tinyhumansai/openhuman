#!/usr/bin/env bash
# Resolve Node 24+ and Appium for E2E scripts (local nvm or CI PATH).
# shellcheck disable=SC2034
# Outputs: NODE24, APPIUM_BIN (export for callers)

NODE24="$(command -v node 2>/dev/null || true)"
export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
if [ -s "$NVM_DIR/nvm.sh" ]; then
  # shellcheck source=/dev/null
  . "$NVM_DIR/nvm.sh"
  NVM_NODE="$(nvm which 24 2>/dev/null || true)"
  if [ -n "${NVM_NODE:-}" ] && [ -x "$NVM_NODE" ]; then
    NODE24="$NVM_NODE"
  fi
fi

if [ -z "${NODE24:-}" ] || [ ! -x "$NODE24" ]; then
  echo "ERROR: Node.js is required (Node 24+ for Appium v3)." >&2
  exit 1
fi

NODE_MAJOR="$("$NODE24" --version | sed 's/^v//' | cut -d. -f1)"
if [ "${NODE_MAJOR:-0}" -lt 24 ]; then
  echo "ERROR: Node 24+ is required for Appium v3 (found $($NODE24 --version))." >&2
  exit 1
fi

# Prefer the appium binary that lives next to the resolved Node 24 binary.
APPIUM_BIN="$(dirname "$NODE24")/appium"
if [ ! -x "$APPIUM_BIN" ]; then
  APPIUM_BIN="$(command -v appium 2>/dev/null || true)"
fi
if [ ! -x "$APPIUM_BIN" ]; then
  echo "ERROR: appium not found. Install with: npm install -g appium" >&2
  exit 1
fi

export NODE24
export APPIUM_BIN
