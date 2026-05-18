#!/usr/bin/env bash
# cleanup.sh <issue-number>
#
# Clean up worktree for completed issue

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=./lib.sh
source "$here/lib.sh"

if [ -z "${1:-}" ]; then
  echo "Usage: pnpm deep-work cleanup <issue-number>" >&2
  echo ""
  echo "Active worktrees:"
  list_deep_work_worktrees | while IFS='=' read -r key value; do
    if [ "$key" = "issue" ]; then
      issue_num="$value"
    elif [ "$key" = "path" ]; then
      echo "  #$issue_num: $value"
    fi
  done
  exit 1
fi

case "$1" in
  ''|*[!0-9]*)
    echo "[deep-work] issue-number must be numeric, got: $1" >&2
    exit 1
    ;;
esac

issue="$1"

echo "[deep-work] 🧹 Cleaning up worktree for issue #$issue"

# Check if worktree exists
if ! worktree_exists "$issue"; then
  echo "[deep-work] ❌ no worktree found for issue #$issue"
  exit 1
fi

worktree_dir=$(worktree_dir_for_issue "$issue")
echo "[deep-work] worktree location: $worktree_dir"

# Get branch info
branch=""
if [ -d "$worktree_dir" ]; then
  branch=$(cd "$worktree_dir" && git branch --show-current 2>/dev/null || echo "unknown")
  echo "[deep-work] branch: $branch"
fi

# Check for uncommitted changes
if [ -d "$worktree_dir/.git" ]; then
  if ! (cd "$worktree_dir" && git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]); then
    echo ""
    echo "⚠️  WARNING: Worktree has uncommitted changes!"
    echo ""
    echo "Files with changes:"
    (cd "$worktree_dir" && git status --short)
    echo ""
    echo "These changes will be LOST if you proceed with cleanup."
    echo ""
    echo "Options:"
    echo "1) Cancel cleanup and commit changes first"
    echo "2) Continue anyway and lose changes"
    echo ""
    read -p "What would you like to do? [1/2]: " -r choice
    case "$choice" in
      1)
        echo "Cleanup cancelled. Please commit your changes first:"
        echo "  cd $worktree_dir"
        echo "  git add ."
        echo "  git commit -m 'message'"
        echo "  pnpm deep-work cleanup $issue"
        exit 0
        ;;
      2)
        echo "Proceeding with cleanup, changes will be lost..."
        ;;
      *)
        echo "Invalid choice. Cancelling cleanup."
        exit 1
        ;;
    esac
  fi
fi

# Check PR status
repo=$(resolve_deep_work_repo)
pr_status=""
pr_url=""

if [ -n "$branch" ] && [ "$branch" != "unknown" ]; then
  if pr_json=$(gh pr view "$branch" -R "$repo" --json url,state,isDraft,mergeable 2>/dev/null); then
    pr_url=$(echo "$pr_json" | jq -r '.url')
    pr_state=$(echo "$pr_json" | jq -r '.state')
    pr_draft=$(echo "$pr_json" | jq -r '.isDraft')

    if [ "$pr_state" = "MERGED" ]; then
      pr_status="merged"
    elif [ "$pr_draft" = "true" ]; then
      pr_status="draft"
    elif [ "$pr_state" = "OPEN" ]; then
      pr_status="open"
    else
      pr_status="$pr_state"
    fi

    echo ""
    echo "📋 PR Status: $pr_status"
    echo "🔗 PR URL: $pr_url"
  else
    echo ""
    echo "📋 No PR found for branch $branch"
  fi
fi

# Confirm cleanup
echo ""
echo "This will:"
echo "  ✗ Remove worktree directory: $worktree_dir"
echo "  ✗ Delete local branch: $branch"
echo "  ✓ Keep remote branch and PR intact (if they exist)"
echo ""

if [ "$pr_status" = "draft" ] || [ "$pr_status" = "open" ]; then
  echo "⚠️  Note: PR is still $pr_status. You may want to merge it first."
  echo ""
fi

read -p "Continue with cleanup? [y/N]: " -r
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
  echo "Cleanup cancelled."
  exit 0
fi

# Perform cleanup
cleanup_worktree "$issue" "true"

echo ""
echo "[deep-work] ✅ Cleanup complete!"

if [ -n "$pr_url" ]; then
  echo ""
  echo "📋 PR remains available: $pr_url"

  if [ "$pr_status" = "merged" ]; then
    echo ""
    echo "🎉 Issue #$issue has been successfully completed and merged!"
  fi
fi

echo ""
echo "🚀 Ready to start your next deep-work session:"
echo "  pnpm deep-work pick"
echo "  pnpm deep-work start <issue-number>"