#!/usr/bin/env bash
# Load .env file into environment variables and optional ci-secrets for signing/notarization.
# Usage:
#   source scripts/load-env.sh
#   eval "$(scripts/load-env.sh)"

set -e

# source ./load-dotenv.sh

if [[ -f scripts/ci-secrets.local.json ]]; then
  source scripts/load-env-json.sh scripts/ci-secrets.json
  # Tauri notarization expects APPLE_PASSWORD; secrets file uses APPLE_APP_SPECIFIC_PASSWORD
  if [[ -z "${APPLE_PASSWORD:-}" && -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]]; then
    export APPLE_PASSWORD="$APPLE_APP_SPECIFIC_PASSWORD"
  fi
fi
