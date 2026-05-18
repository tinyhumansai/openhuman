#!/usr/bin/env bash
# list.sh
#
# Simple list of all deep-work worktrees

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=./lib.sh
source "$here/lib.sh"

echo "[deep-work] 📂 All deep-work worktrees:"
echo ""

worktrees_found=false

list_deep_work_worktrees | while IFS='=' read -r key value; do
  if [ "$key" = "issue" ]; then
    issue_num="$value"
    worktrees_found=true
  elif [ "$key" = "path" ]; then
    path="$value"
    branch=""
    status=""

    if [ -d "$path" ]; then
      if [ -d "$path/.git" ]; then
        branch=$(cd "$path" && git branch --show-current 2>/dev/null || echo "unknown")

        # Quick status check
        if cd "$path" && git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
          status="clean"
        else
          status="uncommitted changes"
        fi
      else
        status="not a git repo"
      fi
    else
      status="directory missing"
    fi

    printf "  #%-4s %s\n" "$issue_num" "$path"
    printf "         branch: %s\n" "$branch"
    printf "         status: %s\n" "$status"
    echo ""
  fi
done

# Check if we found any worktrees (need to use a different approach since the while loop runs in a subshell)
if ! list_deep_work_worktrees | grep -q "issue="; then
  echo "  No deep-work worktrees found."
  echo ""
  echo "  To create a new one:"
  echo "    pnpm deep-work start <issue-number>"
  echo "    pnpm deep-work pick"
fi