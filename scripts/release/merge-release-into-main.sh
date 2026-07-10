#!/usr/bin/env bash
# Merge the current HEAD of the release checkout back into main and push.
#
# Usage: merge-release-into-main.sh <merge-commit-message>
#
# Called by release-staging.yml and release-production.yml right after the
# version-bump commit (and tag) land on `release`, so main stays in sync with
# everything on release (bump commit + any fix commits). Expects to run in a
# repo checkout whose HEAD is the release commit to merge and whose `origin`
# remote is authenticated for pushing to main (App token remote URL).
#
# Fast-forward is preferred: when main hasn't moved since the promotion cut,
# ff leaves main == release and the next promote-main-to-release.yml dispatch
# sees nothing to promote. Otherwise a --no-ff merge commit with the given
# message is created. Conflicts exit 1 with a warning — callers keep
# continue-on-error so a conflicted back-merge never strands a release that
# is already tagged; resolve the merge manually in that case.
set -euo pipefail

if [ $# -ne 1 ] || [ -z "$1" ]; then
  echo "usage: $0 <merge-commit-message>" >&2
  exit 2
fi
MERGE_MESSAGE="$1"

log() { echo "[release][back-merge] $*"; }

RELEASE_SHA="$(git rev-parse HEAD)"
log "merging release ($RELEASE_SHA) back into main"
git fetch origin main
git checkout -B main origin/main
if git merge --ff-only "$RELEASE_SHA" 2>/dev/null; then
  git push origin HEAD:main
  log "fast-forwarded main to $RELEASE_SHA"
elif git merge --no-ff "$RELEASE_SHA" -m "$MERGE_MESSAGE"; then
  git push origin HEAD:main
  log "merged $RELEASE_SHA into main (merge commit)"
else
  echo "::warning::Automatic release→main back-merge hit conflicts. Merge branch 'release' into 'main' manually."
  exit 1
fi
