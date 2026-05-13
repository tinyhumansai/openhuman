#!/usr/bin/env bash
# coverage.sh <pr-number> [--agent <tool>] [extra-prompt]
# Sync the PR locally, gather coverage-related GitHub Actions context, then hand
# off to an agent to fix coverage gate failures, improve coverage, and babysit
# the PR until the relevant checks are green or the work is clearly blocked.
#
# --agent picks the CLI that drives the work. Default: claude.
# A trailing positional <extra-prompt> (any free-form text) is appended to the
# agent's prompt verbatim.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

require git gh jq
require_pr_number "${1:-}"

pr="$1"
agent="claude"
extra_prompt=""
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --agent) agent="${2:?--agent requires a value}"; shift 2 ;;
    --agent=*) agent="${1#*=}"; shift ;;
    *)
      if [ -n "$extra_prompt" ]; then
        echo "[review] unexpected extra arg: $1 (extra-prompt already set)" >&2
        exit 1
      fi
      extra_prompt="$1"; shift
      ;;
  esac
done

require "$agent"
sync_pr "$pr"

coverage_checks=$(gh pr checks "$REVIEW_PR" -R "$REVIEW_REPO_RESOLVED" 2>/dev/null \
  | grep -i 'coverage' || true)

coverage_runs_json=$(gh run list -R "$REVIEW_REPO_RESOLVED" \
  --workflow "Coverage Gate" \
  --branch "$REVIEW_HEAD_BRANCH" \
  --limit 5 \
  --json databaseId,status,conclusion,url,createdAt,updatedAt,headSha || echo '[]')

coverage_runs_summary=$(printf '%s\n' "$coverage_runs_json" | jq -r '
  if length == 0 then
    "No recent Coverage Gate workflow runs found for this branch."
  else
    .[] | "- run=\(.databaseId) status=\(.status) conclusion=\(.conclusion // "n/a") sha=\(.headSha) updated=\(.updatedAt) url=\(.url)"
  end
')

if [ -z "$coverage_checks" ]; then
  coverage_checks="No current coverage-related checks were returned by gh pr checks."
fi

prompt="I've already checked out branch pr/$REVIEW_PR with main \
merged in and upstream tracking set (repo: $REVIEW_REPO_RESOLVED). Focus on the \
coverage gate for PR #$REVIEW_PR.

Current coverage-related PR checks:
$coverage_checks

Recent Coverage Gate workflow runs for this branch:
$coverage_runs_summary

Use the GitHub Actions error output for the coverage jobs to identify what is \
failing. Fix the coverage workflow or scripts if they are broken, improve test \
coverage as needed to satisfy the gate, run the relevant local checks, and push \
the fixes back to the PR branch.

After pushing, babysit the PR: monitor the coverage and related required checks, \
investigate any new failures, and keep iterating until the coverage gate is \
green or you hit a real blocker. If you are blocked, summarize the blocker \
clearly with the failing check name, exact error, and what remains to fix."

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

Additional instructions from the user:
${extra_prompt}"
fi

"$agent" "$prompt"
