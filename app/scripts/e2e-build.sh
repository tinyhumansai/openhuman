#!/usr/bin/env bash
#
# Build the app for E2E tests with the mock server URL baked in.
#
# - macOS: builds a .app bundle (Appium Mac2)
# - Linux: builds a debug binary (tauri-driver)
#
# Cargo incremental builds are used by default for faster iteration.
#
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$APP_DIR/.." && pwd)"
cd "$APP_DIR"

# Source Cargo environment
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT:-18473}"

echo "Building E2E app with VITE_BACKEND_URL=$VITE_BACKEND_URL"

if [ -n "${E2E_FORCE_CARGO_CLEAN:-}" ]; then
  echo "Forcing cargo clean (E2E_FORCE_CARGO_CLEAN is set)."
  cargo clean --manifest-path src-tauri/Cargo.toml
else
  echo "Skipping cargo clean (default incremental E2E build)."
fi

if [ -f .env ]; then
  # shellcheck source=/dev/null
  source "$REPO_ROOT/scripts/load-dotenv.sh"
else
  echo "No .env file — skipping load-dotenv (optional for CI)."
fi

export VITE_BACKEND_URL="http://127.0.0.1:${E2E_MOCK_PORT:-18473}"

# Core is compiled in-process into the Tauri shell as of PR #1061; the old
# scripts/stage-core-sidecar.mjs staging step is no longer needed.

# Build the frontend in DEV mode up-front so `import.meta.env.DEV` is true
# in the bundled E2E binary. That flips `restartApp` in
# app/src/utils/tauriCommands/core.ts from the real `app.restart()`
# (which destroys the WebDriver CDP target) to a benign
# `window.location.reload()`. Without this, the identity-flip path that
# fires on every E2E login kills the chromium-driver session.
#
# We then override `beforeBuildCommand` to a no-op so Tauri does not
# clobber our dev-mode dist with a fresh prod-mode build.
echo "Building frontend (dev mode) for E2E..."
pnpm run build:app:e2e

TAURI_CONFIG_OVERRIDE='{"bundle":{"createUpdaterArtifacts":false},"build":{"beforeBuildCommand":""}}'
# Tauri CLI maps env CI to --ci and only accepts true|false; some runners set CI=1.
case "${CI:-}" in 1) export CI=true ;; 0) export CI=false ;; esac

# CEF runtime requires the vendored CEF-aware tauri-cli (the stock one produces
# a bundle that panics at startup in cef::library_loader::LibraryLoader::new).
# All other build scripts in app/package.json do `pnpm tauri:ensure` + use
# `cargo tauri build`; the E2E build was the one outlier and we got the panic.
pnpm tauri:ensure
export CEF_PATH="$HOME/Library/Caches/tauri-cef"

OS="$(uname)"
case "$OS" in
  Linux)
    # Linux: build debug binary only.
    echo "Building for Linux (debug binary, no bundle)..."
    cargo tauri build -c "$TAURI_CONFIG_OVERRIDE" --debug --no-bundle --features e2e-test-support -- --bin OpenHuman
    ;;
  Darwin)
    # macOS: build .app bundle (wdio.conf points at
    # src-tauri/target/debug/bundle/macos/OpenHuman.app).
    echo "Building for macOS (.app bundle)..."
    cargo tauri build -c "$TAURI_CONFIG_OVERRIDE" --bundles app --debug --features e2e-test-support -- --bin OpenHuman
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    # Windows: bare .exe at src-tauri/target/debug/OpenHuman.exe.
    echo "Building for Windows (.exe, no bundle)..."
    cargo tauri build -c "$TAURI_CONFIG_OVERRIDE" --debug --no-bundle --features e2e-test-support -- --bin OpenHuman
    ;;
  *)
    echo "ERROR: unsupported OS for e2e build: $OS" >&2
    exit 1
    ;;
esac

echo "E2E build complete."
