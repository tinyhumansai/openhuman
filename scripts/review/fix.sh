#!/usr/bin/env bash
# fix.sh <pr-number> [--executor-llm <tool>]
# Sync the PR, run pr-reviewer to identify issues and apply fixes, then hand
# off to pr-manager-lite to run the quality suite, commit, and push.
#
# --executor-llm picks the CLI that drives the agent. Default: claude.

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
pr-reviewer agent to review PR #$REVIEW_PR and fix the issues it finds. Then \
use the pr-manager-lite agent to run the quality suite, commit, and push the \
changes back to the PR branch."
