#!/usr/bin/env bash
# review.sh <pr-number> [--agent <tool>] [extra-prompt]
# Sync the PR locally, then hand a fully-inlined CodeRabbit-style review prompt
# to the chosen agent. The prompt is loaded from scripts/shortcuts/review/prompts/review.md
# so the workflow is agent-agnostic (no reliance on Claude Code's named
# subagent registry).
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

template="$here/prompts/review.md"
if [ ! -f "$template" ]; then
  echo "[review] missing prompt template: $template" >&2
  exit 1
fi

if [ "${REVIEW_HAS_CONFLICTS:-0}" = "1" ]; then
  conflict_block="# ⚠️ Merge conflicts detected

When the PR branch was merged with current \`main\`, the following files were left with unresolved conflict markers:

$(printf '%s\n' "$REVIEW_CONFLICT_FILES" | sed 's/^/- /')

Since this is a **review-only** run, do NOT resolve them — but you MUST call them out prominently in the review walkthrough as a blocker (with severity 🛑) and request changes on the PR. Tell the author exactly which files need attention before this PR can merge."
else
  conflict_block=""
fi

prompt=$(REVIEW_CONFLICT_BLOCK="$conflict_block" \
         awk -v pr="$REVIEW_PR" -v repo="$REVIEW_REPO_RESOLVED" '
  BEGIN { conflict = ENVIRON["REVIEW_CONFLICT_BLOCK"] }
  { gsub(/__PR__/, pr); gsub(/__REPO__/, repo); gsub(/__CONFLICT_BLOCK__/, conflict); print }
' "$template")

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

# Additional instructions from the user
${extra_prompt}"
fi

"$agent" "$prompt"
