#!/usr/bin/env bash
# review.sh <pr-number> [--executor-llm <tool>] [extra-prompt]
# Sync the PR locally, then hand off to the pr-reviewer agent to produce a
# CodeRabbit-style review, post it, and approve the PR if it looks good.
#
# --executor-llm picks the CLI that drives the agent. Default: claude.
# (Note: the pr-reviewer / pr-manager-lite agents are Claude Code constructs;
# switching executors only makes sense if the alternate CLI understands them.)
# A trailing positional <extra-prompt> (any free-form text) is appended to the
# executor's prompt verbatim.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

require git gh jq
require_pr_number "${1:-}"

pr="$1"
executor_llm="claude"
extra_prompt=""
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --executor-llm) executor_llm="${2:?--executor-llm requires a value}"; shift 2 ;;
    --executor-llm=*) executor_llm="${1#*=}"; shift ;;
    *)
      if [ -n "$extra_prompt" ]; then
        echo "[review] unexpected extra arg: $1 (extra-prompt already set)" >&2
        exit 1
      fi
      extra_prompt="$1"; shift
      ;;
  esac
done

require "$executor_llm"
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

"$executor_llm" "$prompt"
