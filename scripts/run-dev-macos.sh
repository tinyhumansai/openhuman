#!/usr/bin/env bash
# Start the macOS Tauri development app from a clean checkout or a configured
# local environment. A `.env` is optional: local defaults and inherited values
# are sufficient for the desktop dev path.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [[ -f "$REPO_ROOT/.env" ]]; then
  # shellcheck source=load-dotenv.sh
  source "$SCRIPT_DIR/load-dotenv.sh" "$REPO_ROOT/.env"
fi

raw_dev_port="${OPENHUMAN_DEV_PORT:-1420}"
raw_dev_port="${raw_dev_port//[[:space:]]/}"
if [[ "$raw_dev_port" =~ ^[0-9]+$ ]] && (( 10#$raw_dev_port >= 1 && 10#$raw_dev_port <= 65535 )); then
  dev_port="$raw_dev_port"
else
  echo "[run-dev-macos] WARNING: invalid OPENHUMAN_DEV_PORT='$raw_dev_port'; falling back to 1420" >&2
  dev_port=1420
fi

export OPENHUMAN_DEV_PORT="$dev_port"
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-OpenHuman Dev Signer}"
config_override="{\"build\":{\"devUrl\":\"http://localhost:${dev_port}\"}}"

cd "$REPO_ROOT/app"
exec cargo tauri dev --config "$config_override"
