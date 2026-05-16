#!/usr/bin/env bash
# Shared utilities for deep-work scripts

set -euo pipefail

# Source the existing review lib for repo resolution
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
# shellcheck source=../review/lib.sh
source "$repo_root/scripts/review/lib.sh"

# Worktree utilities
worktree_dir_for_issue() {
  local issue="$1"
  echo "../oh-$issue"
}

worktree_branch_for_issue() {
  local issue="$1"
  local title="$2"
  local branch_prefix="${DEEP_WORK_BRANCH_PREFIX:-fix}"

  # Create slug from title
  local slug
  slug=$(printf '%s' "$title" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
    | cut -c1-30 \
    | sed -E 's/-+$//')

  if [ -z "$slug" ]; then
    slug="work"
  fi

  echo "${branch_prefix}/${issue}-${slug}"
}

# Check if worktree exists for issue
worktree_exists() {
  local issue="$1"
  local worktree_dir
  worktree_dir=$(worktree_dir_for_issue "$issue")
  [ -d "$worktree_dir" ]
}

# Get current worktree info
get_current_worktree_info() {
  local current_dir
  current_dir=$(pwd)

  # Check if we're in a worktree by looking for oh-<number> pattern
  if [[ "$current_dir" =~ /oh-([0-9]+)$ ]]; then
    local issue="${BASH_REMATCH[1]}"
    local branch
    branch=$(git branch --show-current 2>/dev/null || echo "")
    echo "issue=$issue branch=$branch dir=$current_dir"
  else
    echo "not_in_worktree"
  fi
}

# Resolve repo (with DEEP_WORK_REPO override)
resolve_deep_work_repo() {
  local repo="${DEEP_WORK_REPO:-${WORK_REPO:-${REVIEW_REPO:-}}}"
  if [ -z "$repo" ]; then
    repo=$(REVIEW_REPO= resolve_repo)
  fi
  echo "$repo"
}

# Create worktree for issue
create_worktree() {
  local issue="$1"
  local title="$2"

  local worktree_dir
  worktree_dir=$(worktree_dir_for_issue "$issue")

  local branch
  branch=$(worktree_branch_for_issue "$issue" "$title")

  if worktree_exists "$issue"; then
    echo "[deep-work] worktree for issue #$issue already exists at $worktree_dir"
    return 1
  fi

  echo "[deep-work] creating worktree at $worktree_dir with branch $branch..."
  git worktree add "$worktree_dir" -b "$branch"

  echo "[deep-work] worktree created: $worktree_dir"
  echo "[deep-work] branch: $branch"
}

# List all deep-work worktrees
list_deep_work_worktrees() {
  git worktree list --porcelain | while IFS= read -r line; do
    if [[ "$line" =~ ^worktree\ (.*/oh-([0-9]+))$ ]]; then
      local path="${BASH_REMATCH[1]}"
      local issue="${BASH_REMATCH[2]}"
      echo "issue=$issue path=$path"
    fi
  done
}

# Clean up worktree
cleanup_worktree() {
  local issue="$1"
  local force="${2:-false}"

  local worktree_dir
  worktree_dir=$(worktree_dir_for_issue "$issue")

  if ! worktree_exists "$issue"; then
    echo "[deep-work] no worktree found for issue #$issue"
    return 1
  fi

  if [ "$force" != "true" ]; then
    echo "This will remove the worktree at $worktree_dir and delete the branch."
    echo "Any uncommitted changes will be lost."
    read -p "Are you sure? (y/N) " -r
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
      echo "Cancelled."
      return 1
    fi
  fi

  # Get branch name before removing worktree
  local branch
  if [ -d "$worktree_dir" ]; then
    branch=$(cd "$worktree_dir" && git branch --show-current 2>/dev/null || echo "")
  fi

  echo "[deep-work] removing worktree $worktree_dir..."
  git worktree remove "$worktree_dir" --force

  if [ -n "$branch" ] && git show-ref --verify --quiet "refs/heads/$branch"; then
    echo "[deep-work] deleting branch $branch..."
    git branch -D "$branch" || echo "[deep-work] failed to delete branch $branch"
  fi

  echo "[deep-work] cleanup completed for issue #$issue"
}

# Sync with upstream
sync_upstream() {
  echo "[deep-work] syncing with upstream..."

  # Ensure we're on main
  if [ "$(git branch --show-current)" != "main" ]; then
    echo "[deep-work] switching to main branch..."
    git checkout main
  fi

  # Fetch and merge upstream
  if git remote get-url upstream >/dev/null 2>&1; then
    git fetch upstream
    echo "[deep-work] merging upstream/main..."
    git merge --ff-only upstream/main || git merge upstream/main
  fi

  # Update from origin
  if git remote get-url origin >/dev/null 2>&1; then
    echo "[deep-work] pulling from origin/main..."
    git pull --ff-only origin main
  fi

  # Update submodules
  git submodule update --init --recursive

  echo "[deep-work] upstream sync completed"
}

# Run quality checks
run_quality_checks() {
  local step="$1"
  echo "[deep-work] running quality checks for $step..."

  # Typecheck
  echo "[deep-work] typechecking..."
  if ! pnpm typecheck; then
    echo "[deep-work] ❌ typecheck failed"
    return 1
  fi

  # Lint
  echo "[deep-work] linting..."
  if ! pnpm lint; then
    echo "[deep-work] ❌ lint failed"
    return 1
  fi

  # Format check
  echo "[deep-work] format check..."
  if ! pnpm format:check; then
    echo "[deep-work] running formatter..."
    pnpm format
  fi

  # Build
  echo "[deep-work] building..."
  if ! pnpm build; then
    echo "[deep-work] ❌ build failed"
    return 1
  fi

  echo "[deep-work] ✅ all quality checks passed"
}