#!/usr/bin/env bash
# review.sh <pr-number> [--executor-llm <tool>]
# Sync the PR locally, then hand off to the pr-reviewer agent to produce a
# CodeRabbit-style review, post it, and approve the PR if it looks good.
#
# --executor-llm picks the CLI that drives the agent. Default: claude.
# (Note: the pr-reviewer / pr-manager-lite agents are Claude Code constructs;
# switching executors only makes sense if the alternate CLI understands them.)

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

require git gh jq
require_pr_number "${1:-}"

pr="$1"
executor_llm="claude"
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --executor-llm) executor_llm="${2:?--executor-llm requires a value}"; shift 2 ;;
    --executor-llm=*) executor_llm="${1#*=}"; shift ;;
    *) echo "[review] unknown arg: $1" >&2; exit 1 ;;
  esac
done

require "$executor_llm"
sync_pr "$pr"

"$executor_llm" "I've already checked out branch pr/$REVIEW_PR with main \
merged in and upstream tracking set (repo: $REVIEW_REPO_RESOLVED). Use the \
pr-reviewer agent to produce a CodeRabbit-style review of PR #$REVIEW_PR and \
publish review comments. After the review is posted and if the changes look \
acceptable overall, approve the PR with \`gh pr review $REVIEW_PR -R \
$REVIEW_REPO_RESOLVED --approve\`. If blocking issues remain, request changes \
instead of approving."
