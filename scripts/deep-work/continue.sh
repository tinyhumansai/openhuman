#!/usr/bin/env bash
# continue.sh [issue-number]
#
# Resume deep-work workflow from current state

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=./lib.sh
source "$here/lib.sh"

issue="${1:-}"

# If no issue provided, try to detect from current directory
if [ -z "$issue" ]; then
  worktree_info=$(get_current_worktree_info)
  if [ "$worktree_info" = "not_in_worktree" ]; then
    echo "[deep-work] not currently in a worktree and no issue specified"
    echo ""
    echo "Usage: pnpm deep-work continue [issue-number]"
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

  # Extract issue from worktree info
  issue=$(echo "$worktree_info" | grep -o 'issue=[0-9]*' | cut -d= -f2)
  echo "[deep-work] detected issue #$issue from current directory"
fi

# Validate issue number
case "$issue" in
  ''|*[!0-9]*)
    echo "[deep-work] invalid issue number: $issue" >&2
    exit 1
    ;;
esac

# Check if worktree exists
worktree_dir=$(worktree_dir_for_issue "$issue")
if ! worktree_exists "$issue"; then
  echo "[deep-work] no worktree found for issue #$issue"
  echo "[deep-work] use 'pnpm deep-work start $issue' to begin work"
  exit 1
fi

echo "[deep-work] 🔄 Resuming work on issue #$issue"

# Change to worktree
cd "$worktree_dir"
echo "[deep-work] 📁 working in: $(pwd)"
echo "[deep-work] 🌿 branch: $(git branch --show-current)"

# Check git status to determine next step
git_status=$(git status --porcelain)
has_staged=$(git diff --cached --quiet && echo "false" || echo "true")
has_unstaged=$(git diff --quiet && echo "false" || echo "true")
has_untracked=$([ -z "$(git ls-files --others --exclude-standard)" ] && echo "false" || echo "true")

echo ""
echo "[deep-work] analyzing current state..."

# Determine what step to resume from
if [ "$has_staged" = "true" ] || [ "$has_unstaged" = "true" ] || [ "$has_untracked" = "true" ]; then
  echo "[deep-work] 📝 detected uncommitted changes"
  echo ""
  echo "Git status:"
  git status --short
  echo ""
  echo "Next steps:"
  echo "1) Continue implementation - call codecrusher to finish work"
  echo "2) Run quality checks - typecheck, lint, format, build"
  echo "3) Commit changes - stage and commit current work"
  echo "4) Manual review - inspect changes yourself"
  echo ""

  while true; do
    read -p "What would you like to do? [1-4]: " -r choice
    case "$choice" in
      1)
        echo "[deep-work] 💻 calling codecrusher to continue implementation..."

        repo=$(resolve_deep_work_repo)
        issue_json=$(gh issue view "$issue" -R "$repo" --json title,body,url)
        title=$(jq -r '.title' <<<"$issue_json")
        body=$(jq -r '.body // ""' <<<"$issue_json")
        url=$(jq -r '.url' <<<"$issue_json")

        implementation_prompt="Continue working on GitHub issue #${issue} from previous session.

Working branch: $(git branch --show-current)
Issue URL: ${url}
Issue title: ${title}

Current uncommitted changes detected. Please review the current state and continue implementation:

--- Issue body ---
${body}
--- end issue body ---

Complete the implementation following the existing patterns and ensure all requirements are met."

        claude "$implementation_prompt" --task codecrusher
        break
        ;;
      2)
        echo "[deep-work] 🔍 running quality checks..."
        if run_quality_checks "continue"; then
          echo "[deep-work] ✅ quality checks passed"
        else
          echo "[deep-work] ❌ quality checks failed - please fix issues"
        fi
        break
        ;;
      3)
        echo "[deep-work] 📝 preparing to commit..."
        git add .

        repo=$(resolve_deep_work_repo)
        issue_json=$(gh issue view "$issue" -R "$repo" --json title,body)
        title=$(jq -r '.title' <<<"$issue_json")
        body=$(jq -r '.body // ""' <<<"$issue_json")

        commit_message="fix(#${issue}): ${title}

${body}

🤖 Generated with [Claude Code](https://claude.ai/code)

Co-Authored-By: Claude <noreply@anthropic.com>"

        git commit -m "$(cat <<EOF
$commit_message
EOF
)"
        echo "[deep-work] ✅ changes committed"
        break
        ;;
      4)
        echo "[deep-work] 👀 opening files for manual review..."
        echo ""
        echo "Modified files:"
        git diff --name-only HEAD
        git diff --name-only --cached
        git ls-files --others --exclude-standard
        echo ""
        echo "Review complete. Run 'pnpm deep-work continue $issue' when ready to proceed."
        exit 0
        ;;
      *)
        echo "Please enter 1, 2, 3, or 4"
        continue
        ;;
    esac
  done
else
  # Clean working tree - check if we need to push/PR
  echo "[deep-work] 📋 working tree is clean"

  # Check if we have unpushed commits
  branch=$(git branch --show-current)
  if git log origin/"$branch"..HEAD --oneline 2>/dev/null | grep -q .; then
    echo "[deep-work] 📤 found unpushed commits"
    echo ""
    echo "Unpushed commits:"
    git log origin/"$branch"..HEAD --oneline
    echo ""
    echo "Next steps:"
    echo "1) Push and create PR"
    echo "2) Run additional quality checks first"
    echo "3) Update memory with learnings"
    echo ""

    while true; do
      read -p "What would you like to do? [1-3]: " -r choice
      case "$choice" in
        1)
          echo "[deep-work] 📤 pushing branch and creating PR..."
          git push -u origin "$branch"

          # Create PR if it doesn't exist
          repo=$(resolve_deep_work_repo)
          if ! gh pr view "$branch" -R "$repo" >/dev/null 2>&1; then
            issue_json=$(gh issue view "$issue" -R "$repo" --json title,body)
            title=$(jq -r '.title' <<<"$issue_json")
            body=$(jq -r '.body // ""' <<<"$issue_json")

            pr_body="## Summary

Closes #${issue}

${body}

🤖 Generated with [Claude Code](https://claude.ai/code)"

            pr_url=$(gh pr create \
              --title "fix(#${issue}): ${title}" \
              --body "$pr_body" \
              --draft \
              --head "$(gh auth status 2>&1 | grep 'Logged in.*as' | sed 's/.*as //' | cut -d' ' -f1):$branch" \
              --base main \
              --repo "$repo")

            echo "[deep-work] 📝 PR created: $pr_url"
          else
            echo "[deep-work] ✅ PR already exists"
          fi
          break
          ;;
        2)
          if run_quality_checks "continue"; then
            echo "[deep-work] ✅ quality checks passed"
          else
            echo "[deep-work] ❌ quality checks failed - please fix issues"
          fi
          break
          ;;
        3)
          echo "[deep-work] 🧠 calling memory-keeper..."
          repo=$(resolve_deep_work_repo)
          issue_json=$(gh issue view "$issue" -R "$repo" --json title)
          title=$(jq -r '.title' <<<"$issue_json")

          memory_prompt="Update project memory with learnings from issue #${issue}: \"${title}\".

Review the recent commits and changes to extract useful insights for future development."

          claude "$memory_prompt" --task memory-keeper
          break
          ;;
        *)
          echo "Please enter 1, 2, or 3"
          continue
          ;;
      esac
    done
  else
    echo "[deep-work] 📋 everything up to date"
    echo ""
    echo "Next steps:"
    echo "1) Check PR status and reviews"
    echo "2) Run final quality checks"
    echo "3) Clean up worktree"
    echo ""

    while true; do
      read -p "What would you like to do? [1-3]: " -r choice
      case "$choice" in
        1)
          repo=$(resolve_deep_work_repo)
          if gh pr view "$branch" -R "$repo" >/dev/null 2>&1; then
            echo "[deep-work] 📋 PR status:"
            gh pr view "$branch" -R "$repo"
          else
            echo "[deep-work] ❌ no PR found for this branch"
          fi
          break
          ;;
        2)
          if run_quality_checks "final"; then
            echo "[deep-work] ✅ final quality checks passed"
          else
            echo "[deep-work] ❌ quality checks failed"
          fi
          break
          ;;
        3)
          echo "[deep-work] 🧹 cleaning up worktree..."
          cd "$(git rev-parse --show-toplevel)/.."
          cleanup_worktree "$issue"
          break
          ;;
        *)
          echo "Please enter 1, 2, or 3"
          continue
          ;;
      esac
    done
  fi
fi

echo ""
echo "[deep-work] 🎯 Continue session complete!"
echo "[deep-work] Use 'pnpm deep-work continue $issue' to resume anytime."