#!/usr/bin/env bash
# review.sh <pr-number> [--agent <tool>] [extra-prompt]
# Sync the PR locally, then hand off to the pr-reviewer agent to produce a
# CodeRabbit-style review, post it, and approve the PR if it looks good.
#
# --agent picks the CLI that drives the work. Default: claude.
# (Note: the pr-reviewer / pr-manager-lite agents are Claude Code constructs;
# switching agents only makes sense if the alternate CLI understands them.)
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
pr-reviewer agent to produce a CodeRabbit-style review of PR #$REVIEW_PR and \
publish review comments. After the review is posted and if the changes look \
acceptable overall, approve the PR with \`gh pr review $REVIEW_PR -R \
$REVIEW_REPO_RESOLVED --approve\`. If blocking issues remain, request changes \
instead of approving."

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

Additional instructions from the user:
${extra_prompt}"
fi

"$agent" "$prompt"
