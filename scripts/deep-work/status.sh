#!/usr/bin/env bash
# status.sh
#
# Show status of all deep-work worktrees and progress

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=./lib.sh
source "$here/lib.sh"

require git gh jq

repo=$(resolve_deep_work_repo)

echo "[deep-work] 📊 Deep Work Status Report"
echo "Repository: $repo"
echo ""

# Get current location info
current_info=$(get_current_worktree_info)
if [ "$current_info" != "not_in_worktree" ]; then
  current_issue=$(echo "$current_info" | grep -o 'issue=[0-9]*' | cut -d= -f2)
  current_branch=$(echo "$current_info" | grep -o 'branch=[^[:space:]]*' | cut -d= -f2)
  current_dir=$(echo "$current_info" | grep -o 'dir=[^[:space:]]*' | cut -d= -f2)
  echo "📍 Current location: Issue #$current_issue (branch: $current_branch)"
  echo "   Directory: $current_dir"
  echo ""
fi

# List all deep-work worktrees
worktrees_data=""
while IFS='=' read -r key value; do
  if [ "$key" = "issue" ]; then
    current_issue_num="$value"
  elif [ "$key" = "path" ]; then
    current_path="$value"
    worktrees_data+="$current_issue_num|$current_path\n"
  fi
done < <(list_deep_work_worktrees)

if [ -z "$worktrees_data" ]; then
  echo "🏜️  No active deep-work sessions found."
  echo ""
  echo "To start working on an issue:"
  echo "  pnpm deep-work start <issue-number>"
  echo "  pnpm deep-work pick"
  exit 0
fi

echo "🚀 Active deep-work sessions:"
echo ""

printf "%-6s %-20s %-15s %-30s %s\n" "ISSUE" "STATUS" "BRANCH" "TITLE" "PROGRESS"
echo "────────────────────────────────────────────────────────────────────────────────────"

echo -e "$worktrees_data" | while IFS='|' read -r issue_num path; do
  if [ -z "$issue_num" ] || [ -z "$path" ]; then
    continue
  fi

  if [ ! -d "$path" ]; then
    printf "%-6s %-20s %-15s %-30s %s\n" "#$issue_num" "❌ MISSING" "?" "?" "Worktree directory not found"
    continue
  fi

  # Get branch and git status from worktree
  branch=""
  git_clean=""
  has_commits=""
  pr_exists=""

  if [ -d "$path/.git" ]; then
    branch=$(cd "$path" && git branch --show-current 2>/dev/null || echo "unknown")

    # Check if working tree is clean
    if cd "$path" && git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
      git_clean="clean"
    else
      git_clean="dirty"
    fi

    # Check for commits ahead of origin
    if cd "$path" && git log origin/"$branch"..HEAD --oneline 2>/dev/null | grep -q .; then
      has_commits="unpushed"
    elif cd "$path" && git log --oneline -1 >/dev/null 2>&1; then
      has_commits="pushed"
    else
      has_commits="no_commits"
    fi

    # Check if PR exists
    if gh pr view "$branch" -R "$repo" >/dev/null 2>&1; then
      pr_status=$(gh pr view "$branch" -R "$repo" --json state,isDraft | jq -r 'if .isDraft then "draft" else .state end')
      pr_exists="$pr_status"
    else
      pr_exists="no_pr"
    fi
  fi

  # Get issue title
  issue_title=""
  if issue_json=$(gh issue view "$issue_num" -R "$repo" --json title 2>/dev/null); then
    issue_title=$(echo "$issue_json" | jq -r '.title' | cut -c1-30)
  else
    issue_title="(issue not found)"
  fi

  # Determine overall status
  status=""
  progress=""

  if [ "$git_clean" = "dirty" ]; then
    status="🛠️  WORKING"
    progress="Implementation in progress"
  elif [ "$has_commits" = "no_commits" ]; then
    status="🆕 FRESH"
    progress="Just started"
  elif [ "$has_commits" = "unpushed" ]; then
    status="📝 COMMITTED"
    progress="Ready to push"
  elif [ "$pr_exists" = "no_pr" ]; then
    status="📤 PUSHED"
    progress="Ready for PR"
  elif [ "$pr_exists" = "draft" ]; then
    status="📋 DRAFT_PR"
    progress="PR created, needs review"
  elif [ "$pr_exists" = "OPEN" ]; then
    status="🔍 IN_REVIEW"
    progress="Under review"
  elif [ "$pr_exists" = "MERGED" ]; then
    status="✅ MERGED"
    progress="Complete, ready for cleanup"
  else
    status="❓ UNKNOWN"
    progress="Unknown state"
  fi

  printf "%-6s %-20s %-15s %-30s %s\n" "#$issue_num" "$status" "$branch" "$issue_title" "$progress"
done

echo ""
echo "📋 Legend:"
echo "  🆕 FRESH        - Just started, no commits yet"
echo "  🛠️  WORKING      - Implementation in progress (uncommitted changes)"
echo "  📝 COMMITTED    - Work committed locally, ready to push"
echo "  📤 PUSHED       - Pushed to remote, ready to create PR"
echo "  📋 DRAFT_PR     - Draft PR created, under development"
echo "  🔍 IN_REVIEW    - PR open and ready for review"
echo "  ✅ MERGED       - PR merged, ready for cleanup"
echo ""
echo "🛠️  Commands:"
echo "  pnpm deep-work continue <issue>     - Resume work on specific issue"
echo "  pnpm deep-work cleanup <issue>      - Clean up completed worktree"
echo "  pnpm deep-work start <issue>        - Start new issue"
echo "  pnpm deep-work pick                 - Pick and start new issue"