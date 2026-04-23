#!/usr/bin/env bash
# fix.sh <pr-number> [--executor-llm <tool>] [extra-prompt]
# Sync the PR, run pr-reviewer to identify issues and apply fixes, then hand
# off to pr-manager-lite to run the quality suite, commit, and push.
#
# --executor-llm picks the CLI that drives the agent. Default: claude.
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
pr-reviewer agent to review PR #$REVIEW_PR and fix the issues it finds. Then \
use the pr-manager-lite agent to run the quality suite, commit, and push the \
changes back to the PR branch."

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

Additional instructions from the user:
${extra_prompt}"
fi

"$executor_llm" "$prompt"
