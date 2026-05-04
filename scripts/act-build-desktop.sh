#!/usr/bin/env bash
# Run just the reusable build-desktop.yml workflow under act, against an
# existing staging tag. Lets us iterate on the build matrix without
# re-running prepare-build (which would push another bump commit + tag
# to upstream main on every invocation).
#
# Usage:
#   bash scripts/act-build-desktop.sh <staging-tag> [extra act args]
# Example:
#   bash scripts/act-build-desktop.sh v0.53.6-staging --matrix settings.platform:ubuntu-22.04
set -euo pipefail

TAG="${1:-}"
if [ -z "$TAG" ]; then
  echo "Usage: bash scripts/act-build-desktop.sh <staging-tag> [extra act args]" >&2
  exit 1
fi
shift

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS_JSON="${ROOT}/scripts/ci-secrets.json"
SECRETS_FILE="${ROOT}/.secrets"
VARS_FILE="${ROOT}/.vars"
EVENT_FILE="${ROOT}/.github/act-event.json"
ACTRC_FILE="${ROOT}/.actrc"

if [ ! -f "$SECRETS_JSON" ]; then
  echo "Missing $SECRETS_JSON" >&2
  exit 1
fi

# Reuse the dotenv emit + actrc + token translation from act-staging.sh by
# delegating its setup half (it generates everything before the final
# `exec act ...`). Easier than duplicating: source it, but stop before exec.
# Quick hack: run the helper with --list to populate the files, then
# discard its output.
bash "${ROOT}/scripts/act-staging.sh" --list >/dev/null 2>&1 || true

VERSION="${TAG#v}"
VERSION="${VERSION%-staging}"
SHA="$(git ls-remote https://github.com/tinyhumansai/openhuman "refs/tags/$TAG" | awk '{print $1}')"
if [ -z "$SHA" ]; then
  echo "Tag $TAG not found on tinyhumansai/openhuman" >&2
  exit 1
fi
SHORT_SHA="${SHA:0:12}"

echo "[act-build-desktop] tag=$TAG sha=$SHA version=$VERSION"

# build-desktop.yml is `workflow_call`-only; act supports invoking it
# directly via the workflow_call event.
cat > "$EVENT_FILE" <<JSON
{
  "inputs": {
    "build_ref": "${TAG}",
    "tag": "${TAG}",
    "version": "${VERSION}",
    "sha": "${SHA}",
    "short_sha": "${SHORT_SHA}",
    "base_url": "https://staging-api.tinyhumans.ai/",
    "app_env": "staging",
    "build_profile": "debug",
    "telegram_bot_username": "alphahumantest_bot",
    "with_macos_signing": false,
    "with_release_upload": false,
    "with_updater": false,
    "build_sidecar": false
  }
}
JSON

GH_AUTH_TOKEN="$(gh auth token 2>/dev/null || true)"
if [ -n "$GH_AUTH_TOKEN" ]; then
  export GITHUB_TOKEN="$GH_AUTH_TOKEN"
fi

exec act workflow_call \
  -W "${ROOT}/.github/workflows/build-desktop.yml" \
  --eventpath "$EVENT_FILE" \
  --secret-file "$SECRETS_FILE" \
  --var-file "$VARS_FILE" \
  --env GITHUB_REPOSITORY=tinyhumansai/openhuman \
  --env GITHUB_REPOSITORY_OWNER=tinyhumansai \
  "$@"
