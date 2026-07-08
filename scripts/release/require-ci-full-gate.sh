#!/usr/bin/env bash
# Require a successful "CI Full Gate" check run before cutting a release.
#
# Usage: require-ci-full-gate.sh <sha>
#
# Called by release-staging.yml / release-production.yml on the commit they
# are about to bump/tag/build. Direct App-token pushes bypass the PR merge
# gate, so without this check a maintainer could cut a release while CI Full
# is still pending or after it failed on release HEAD.
#
# Version-bump commits carry [skip ci] and therefore never get a CI Full run,
# so first-parent ancestry is walked past them (bounded) to the most recent
# commit that should have one. Needs GH_TOKEN with checks:read and
# GITHUB_REPOSITORY set (both standard in Actions).
set -euo pipefail

SHA="${1:?usage: require-ci-full-gate.sh <sha>}"
CHECK_NAME="CI Full Gate"
MAX_SKIP_DEPTH=10

log() { echo "[release][ci-full-gate] $*"; }

target="$SHA"
depth=0
while [ "$depth" -le "$MAX_SKIP_DEPTH" ]; do
  subject="$(git log -1 --format=%s "$target")"
  if [[ "$subject" != *"[skip ci]"* ]]; then
    break
  fi
  log "$target is a [skip ci] commit ('$subject') — checking its first parent instead"
  target="$(git rev-parse "${target}^")"
  depth=$((depth + 1))
done
if [ "$depth" -gt "$MAX_SKIP_DEPTH" ]; then
  echo "::error::Walked ${MAX_SKIP_DEPTH} [skip ci] commits from ${SHA} without finding a CI-validated commit — something is off with the release history."
  exit 1
fi

log "requiring a successful '${CHECK_NAME}' check run on ${target}"
result="$(gh api -X GET \
  "repos/${GITHUB_REPOSITORY}/commits/${target}/check-runs" \
  -f check_name="${CHECK_NAME}" -f filter=latest \
  --jq '[(.check_runs[0].status // "missing"), (.check_runs[0].conclusion // "none")] | join(" ")')"
status="${result%% *}"
conclusion="${result##* }"
log "status=${status} conclusion=${conclusion}"

if [ "${status}" != "completed" ] || [ "${conclusion}" != "success" ]; then
  echo "::error::'${CHECK_NAME}' on ${target} is ${status}/${conclusion} — cut releases only from a commit with a green CI Full run (re-run ci-full.yml if needed). The skip_ci_gate input overrides this for operator recovery only."
  exit 1
fi
log "'${CHECK_NAME}' is green on ${target}"
