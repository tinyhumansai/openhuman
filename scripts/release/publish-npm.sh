#!/usr/bin/env bash
# Publish the openhuman npm package for a given version.
#
# Usage:
#   publish-npm.sh <tag>
#
# Required environment:
#   NODE_AUTH_TOKEN — npm automation token
#
# Optional environment:
#   DRY_RUN — set to "true" to run npm publish --dry-run
set -euo pipefail

TAG="${1:?Usage: publish-npm.sh <tag>}"
VERSION="${TAG#v}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

cd "$REPO_ROOT/packages/npm"

# Stamp version without creating a git commit
npm version "$VERSION" --no-git-tag-version

PUBLISH_ARGS=(--access public)
if [[ "${DRY_RUN:-}" == "true" ]]; then
  PUBLISH_ARGS+=(--dry-run)
fi

# SKIP_OPENHUMAN_BINARY_DOWNLOAD prevents postinstall from running
# during publish (the binary doesn't exist yet on the publish runner)
SKIP_OPENHUMAN_BINARY_DOWNLOAD=1 npm publish "${PUBLISH_ARGS[@]}"

echo "[npm] Published openhuman@${VERSION}"
