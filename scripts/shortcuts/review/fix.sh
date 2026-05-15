#!/usr/bin/env bash
# fix.sh <pr-number> [--agent <tool>] [extra-prompt]
# Sync the PR locally, then hand a fully-inlined "review + fix + push" prompt
# to the chosen agent. The prompt is loaded from scripts/shortcuts/review/prompts/fix.md
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

template="$here/prompts/fix.md"
if [ ! -f "$template" ]; then
  echo "[review] missing prompt template: $template" >&2
  exit 1
fi

if [ "${REVIEW_HAS_CONFLICTS:-0}" = "1" ]; then
  conflict_block="# ⚠️ Merge conflicts to resolve FIRST

When the PR branch was merged with current \`main\`, the following files were left with unresolved conflict markers (\`<<<<<<<\` / \`=======\` / \`>>>>>>>\`):

$(printf '%s\n' "$REVIEW_CONFLICT_FILES" | sed 's/^/- /')

Before doing anything else:

1. Read each conflicted file and understand both sides — read surrounding code, recent commits on \`main\`, and the PR's intent before resolving.
2. Resolve the conflicts by choosing the correct combination of both sides (NOT \`--ours\` / \`--theirs\` blanket strategies and NOT \`git rebase --skip\`).
3. \`git add\` the resolved files and finish the merge with a clear merge commit: \`git commit --no-edit\` (the conflict-resolution merge message is already staged) — or supply a one-line message if you prefer.
4. Run formatters / typecheck / tests on the resolved files to confirm nothing regressed.

Only after conflicts are cleanly resolved should you proceed to the review/fix workflow below."
else
  conflict_block=""
fi

prompt=$(REVIEW_CONFLICT_BLOCK="$conflict_block" \
         awk -v pr="$REVIEW_PR" -v repo="$REVIEW_REPO_RESOLVED" \
             -v head_repo="$REVIEW_HEAD_REPO" -v head_branch="$REVIEW_HEAD_BRANCH" '
  BEGIN { conflict = ENVIRON["REVIEW_CONFLICT_BLOCK"] }
  {
    gsub(/__PR__/, pr);
    gsub(/__REPO__/, repo);
    gsub(/__HEAD_REPO__/, head_repo);
    gsub(/__HEAD_BRANCH__/, head_branch);
    gsub(/__CONFLICT_BLOCK__/, conflict);
    print
  }
' "$template")

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

# Additional instructions from the user
${extra_prompt}"
fi

"$agent" "$prompt"
