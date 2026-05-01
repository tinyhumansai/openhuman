#!/usr/bin/env bash
# Resolve the git ref a production release should build from.
#
# Inputs (env):
#   RELEASE_SOURCE   `staging_tag` (default) | `main_head`
#   STAGING_TAG      optional: a specific `staging/v*` tag to pin to.
#                    Ignored when RELEASE_SOURCE=main_head.
#                    When RELEASE_SOURCE=staging_tag and empty, the latest
#                    `staging/v*` tag is selected by `git tag --sort=-creatordate`.
#
# Outputs (appended to GITHUB_OUTPUT, also echoed):
#   ref     the resolved tag name or branch (e.g. `staging/v0.53.4-2` or `main`)
#   sha     the resolved full commit SHA
#   source  echoes RELEASE_SOURCE for downstream visibility
#
# Fails loudly when:
#   - RELEASE_SOURCE is unrecognized
#   - staging_tag is requested but no staging tag exists / the named one is missing

set -euo pipefail

RELEASE_SOURCE="${RELEASE_SOURCE:-staging_tag}"
STAGING_TAG="${STAGING_TAG:-}"

case "$RELEASE_SOURCE" in
  staging_tag|main_head) ;;
  *)
    echo "[resolve-release-source] unknown RELEASE_SOURCE='$RELEASE_SOURCE' (expected staging_tag|main_head)" >&2
    exit 1
    ;;
esac

git fetch --tags --quiet origin

if [ "$RELEASE_SOURCE" = "main_head" ]; then
  REF="main"
  SHA="$(git rev-parse origin/main)"
else
  if [ -z "$STAGING_TAG" ]; then
    STAGING_TAG="$(git tag --list 'staging/v*' --sort=-creatordate | head -n 1)"
  fi
  if [ -z "$STAGING_TAG" ]; then
    echo "[resolve-release-source] no staging tags found matching 'staging/v*' — push a staging cut first or rerun with release_source=main_head" >&2
    exit 1
  fi
  if ! git rev-parse --verify --quiet "refs/tags/${STAGING_TAG}" >/dev/null; then
    echo "[resolve-release-source] staging tag '${STAGING_TAG}' does not exist on this remote" >&2
    exit 1
  fi
  REF="$STAGING_TAG"
  SHA="$(git rev-parse "refs/tags/${STAGING_TAG}^{commit}")"
fi

echo "[resolve-release-source] source=$RELEASE_SOURCE ref=$REF sha=$SHA" >&2

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "ref=$REF"
    echo "sha=$SHA"
    echo "source=$RELEASE_SOURCE"
  } >> "$GITHUB_OUTPUT"
fi

echo "ref=$REF"
echo "sha=$SHA"
echo "source=$RELEASE_SOURCE"
