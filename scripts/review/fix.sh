#!/usr/bin/env bash
# fix.sh <pr-number> [--agent <tool>] [extra-prompt]
# Sync the PR, run pr-reviewer to identify issues and apply fixes, then hand
# off to pr-manager-lite to run the quality suite, commit, and push.
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

prompt="I've already checked out branch pr/$REVIEW_PR with main \
merged in and upstream tracking set (repo: $REVIEW_REPO_RESOLVED). Use the \
pr-reviewer agent to review PR #$REVIEW_PR and fix the issues it finds. Then \
use the pr-manager-lite agent to run the quality suite, commit, and push the \
changes back to the PR branch."

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

Additional instructions from the user:
${extra_prompt}"
fi

"$agent" "$prompt"
