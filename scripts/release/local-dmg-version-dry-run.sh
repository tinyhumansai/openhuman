#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_DIR="$REPO_ROOT/app"
APP_BUNDLE="$REPO_ROOT/app/src-tauri/target/release/bundle/macos/OpenHuman.app"
DMG_DIR="$REPO_ROOT/app/src-tauri/target/release/bundle/dmg"
TEMP_ENV_CREATED=0
TMP_TAURI_CONF=""

cleanup() {
  if [[ "$TEMP_ENV_CREATED" -eq 1 ]]; then
    rm -f "$REPO_ROOT/.env"
  fi
  if [[ -n "$TMP_TAURI_CONF" ]]; then
    rm -f "$TMP_TAURI_CONF"
  fi
}
trap cleanup EXIT

echo "[dry-run] Verifying release version files are synced"
node "$REPO_ROOT/scripts/release/verify-version-sync.js"

EXPECTED_VERSION="$(
  node -e 'const fs=require("fs");const p=JSON.parse(fs.readFileSync(process.argv[1], "utf8"));process.stdout.write(p.version);' \
    "$APP_DIR/package.json"
)"
echo "[dry-run] Expected version: $EXPECTED_VERSION"

if [[ ! -f "$REPO_ROOT/.env" ]]; then
  if [[ -f "$REPO_ROOT/.env.example" ]]; then
    cp "$REPO_ROOT/.env.example" "$REPO_ROOT/.env"
    TEMP_ENV_CREATED=1
    echo "[dry-run] Created temporary .env from .env.example for local build"
  else
    echo "[dry-run] ERROR: missing .env and .env.example at repo root" >&2
    exit 1
  fi
fi

HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"

echo "[dry-run] Building frontend bundle"
(
  cd "$APP_DIR"
  npm run build:app
)

echo "[dry-run] Building release openhuman-core for $HOST_TRIPLE"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" --bin openhuman-core

echo "[dry-run] Staging sidecar"
bash "$REPO_ROOT/scripts/release/stage-sidecar.sh" \
  "$HOST_TRIPLE" \
  "target/release" \
  "openhuman-core" \
  "openhuman-core"

TMP_TAURI_CONF="$(mktemp "${TMPDIR:-/tmp}/openhuman-tauri-dry-run.XXXXXX").json"
node -e '
  const fs = require("fs");
  const input = process.argv[1];
  const output = process.argv[2];
  const config = JSON.parse(fs.readFileSync(input, "utf8"));
  config.build = config.build || {};
  config.build.beforeBuildCommand = "echo \"[dry-run] beforeBuildCommand handled externally\"";
  fs.writeFileSync(output, `${JSON.stringify(config, null, 2)}\n`);
' "$APP_DIR/src-tauri/tauri.conf.json" "$TMP_TAURI_CONF"

echo "[dry-run] Building local DMG with staged sidecar"
(
  cd "$APP_DIR"
  source ../scripts/load-dotenv.sh
  yarn tauri build --bundles app,dmg --config "$TMP_TAURI_CONF"
)

if [[ ! -d "$APP_BUNDLE" ]]; then
  echo "[dry-run] ERROR: app bundle not found at $APP_BUNDLE" >&2
  exit 1
fi

if [[ ! -d "$DMG_DIR" ]]; then
  echo "[dry-run] ERROR: DMG directory not found at $DMG_DIR" >&2
  exit 1
fi

DMG_PATH="$(find "$DMG_DIR" -maxdepth 1 -type f -name '*.dmg' | sort | tail -n 1)"
if [[ -z "$DMG_PATH" ]]; then
  echo "[dry-run] ERROR: no DMG artifact produced in $DMG_DIR" >&2
  exit 1
fi

CORE_BIN="$(
  find "$APP_BUNDLE/Contents" -maxdepth 4 -type f -name 'openhuman-core*' ! -name '*.sig' | head -n 1
)"
if [[ -z "$CORE_BIN" ]]; then
  echo "[dry-run] ERROR: packaged openhuman-core binary not found in app bundle" >&2
  exit 1
fi

CORE_VERSION_OUTPUT="$("$CORE_BIN" call --method core.version)"
if ! grep -q "\"version\": \"$EXPECTED_VERSION\"" <<<"$CORE_VERSION_OUTPUT"; then
  echo "[dry-run] ERROR: packaged core version does not match expected version" >&2
  echo "[dry-run] core output: $CORE_VERSION_OUTPUT" >&2
  exit 1
fi

echo "[dry-run] PASS"
echo "[dry-run] DMG: $DMG_PATH"
echo "[dry-run] Core binary: $CORE_BIN"
echo "[dry-run] Core version output: $CORE_VERSION_OUTPUT"
