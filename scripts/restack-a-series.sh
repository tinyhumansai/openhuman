#!/usr/bin/env bash
# Rebase the A-series PR stack (A1 → A2 → A3 → A4) onto origin/main,
# chaining each child onto its parent. Use after an upstream PR merges.
#
# Why this script exists: the A1-A4 PRs are cross-fork PRs targeting
# tinyhumansai/openhuman#main. GitHub's `gh pr edit --base` can't chain
# them because the base branch must live on the upstream repo, and we
# don't push feature branches there. So we keep the linear dependency
# locally: when a parent merges, each child rebases onto the new tip.
#
# Usage:
#   scripts/restack-a-series.sh            # rebase in place, no push
#   scripts/restack-a-series.sh --push     # rebase then force-push each
#   scripts/restack-a-series.sh --stop-at feat/a2-composio-bridge
#
# The stack is hard-coded here in parent→child order. When A5/A7 join
# the chain (post-A4), add them to STACK in the correct position.
#
# Safety: aborts on first rebase conflict. You then resolve manually
# with `git rebase --continue` and re-run the script starting from the
# branch that failed. Nothing is force-pushed until every rebase in the
# run completes cleanly.

set -euo pipefail

STACK=(
  # parent                            child
  "origin/main::feat/life-capture-foundation"   # #817 → main
  "feat/life-capture-foundation::test/imessage-live-e2e"   # A1 → #817
  "test/imessage-live-e2e::feat/a2-composio-bridge"        # A2 → A1
  "feat/a2-composio-bridge::feat/a3-chronicle-dispatcher"  # A3 → A2
  "feat/a3-chronicle-dispatcher::feat/a4-session-manager"  # A4 → A3
)

PUSH=0
STOP_AT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --push) PUSH=1; shift ;;
    --stop-at) STOP_AT="$2"; shift 2 ;;
    -h|--help)
      sed -n '1,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "refusing to restack with a dirty worktree" >&2
  exit 1
fi

ORIGINAL_HEAD=$(git symbolic-ref --short HEAD)
cleanup() { git checkout "$ORIGINAL_HEAD" >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "fetching origin…"
git fetch origin --prune --quiet

for pair in "${STACK[@]}"; do
  parent="${pair%%::*}"
  child="${pair##*::}"
  echo
  echo "── rebasing $child onto $parent ──"

  if ! git rev-parse --verify "$child" >/dev/null 2>&1; then
    echo "  skip: $child not present locally"
    continue
  fi

  git checkout "$child" >/dev/null
  # Try to fast-forward the local parent branch to its remote first so
  # the child rebases onto the latest merged state.
  if [[ "$parent" != origin/* ]] && git rev-parse --verify "origin/$parent" >/dev/null 2>&1; then
    git fetch origin "$parent:$parent" --quiet 2>/dev/null || true
  fi

  if ! git rebase "$parent"; then
    echo >&2
    echo "rebase of $child onto $parent FAILED — resolve manually, then re-run." >&2
    exit 1
  fi

  if [[ "$STOP_AT" == "$child" ]]; then
    echo "stop-at reached: $child"
    break
  fi
done

if [[ $PUSH -eq 1 ]]; then
  echo
  echo "── force-pushing rebased branches ──"
  for pair in "${STACK[@]}"; do
    child="${pair##*::}"
    if git rev-parse --verify "$child" >/dev/null 2>&1; then
      echo "pushing $child"
      git push --force-with-lease origin "$child"
    fi
    if [[ "$STOP_AT" == "$child" ]]; then break; fi
  done
fi

echo
echo "done."
