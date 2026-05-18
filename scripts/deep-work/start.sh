#!/usr/bin/env bash
# start.sh <issue-number>
#
# Full workflow automation for a GitHub issue:
# 0. Worktree setup (create worktree → new branch)
# 1. Issue fetching and validation
# 1.5. Context gathering (CLAUDE.md, memory.md, recent commits)
# 2. Planning with architectobot agent
# 3. Implementation with codecrusher agent
# 4. Cross-checking (typecheck, lint, format, build)
# 5. Quality checks and test runs
# 6. Memory updates with memory-keeper agent
# 7. Commit with proper message
# 8. Merge main and resolve conflicts
# 9. Push and create draft PR
# 10. Review cycle with pr-reviewer agent
# 11. Mark ready for review (with user confirmation)
# 12. Cleanup (with user confirmation)

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"

# shellcheck source=./lib.sh
source "$here/lib.sh"

require git gh jq

if [ -z "${1:-}" ]; then
  echo "Usage: pnpm deep-work start <issue-number>" >&2
  exit 1
fi

case "$1" in
  ''|*[!0-9]*)
    echo "[deep-work] issue-number must be numeric, got: $1" >&2
    exit 1
    ;;
esac

issue="$1"
shift

repo=$(resolve_deep_work_repo)

echo "[deep-work] 🚀 Starting full workflow for issue #$issue from $repo"

# Step 0: Ensure we're on main and synced
echo ""
echo "[deep-work] === Step 0: Upstream sync & worktree setup ==="
sync_upstream

# Step 1: Fetch issue details
echo ""
echo "[deep-work] === Step 1: Fetching issue details ==="
echo "[deep-work] fetching issue #$issue from $repo..."
issue_json=$(gh issue view "$issue" -R "$repo" \
  --json number,title,body,labels,state,url,assignees)

state=$(jq -r '.state' <<<"$issue_json")
if [ "$state" != "OPEN" ]; then
  echo "[deep-work] ⚠️  issue #$issue is $state — continuing anyway" >&2
fi

title=$(jq -r '.title' <<<"$issue_json")
body=$(jq -r '.body // ""' <<<"$issue_json")
url=$(jq -r '.url' <<<"$issue_json")
labels=$(jq -r '[.labels[].name] | join(", ")' <<<"$issue_json")

echo "[deep-work] 📋 Issue: $title"
echo "[deep-work] 🏷️  Labels: ${labels:-(none)}"
echo "[deep-work] 🔗 URL: $url"

# Create worktree
echo ""
echo "[deep-work] creating worktree..."
if ! create_worktree "$issue" "$title"; then
  worktree_dir=$(worktree_dir_for_issue "$issue")
  echo "[deep-work] using existing worktree at $worktree_dir"
fi

# Change to worktree
worktree_dir=$(worktree_dir_for_issue "$issue")
cd "$worktree_dir"
echo "[deep-work] 📁 working in: $(pwd)"
echo "[deep-work] 🌿 branch: $(git branch --show-current)"

# Step 1.5: Context gathering
echo ""
echo "[deep-work] === Step 1.5: Context gathering ==="
echo "[deep-work] reading project documentation..."

context_info=""
if [ -f "CLAUDE.md" ]; then
  echo "[deep-work] ✅ found CLAUDE.md"
  context_info+="✅ CLAUDE.md available\n"
else
  echo "[deep-work] ⚠️  CLAUDE.md not found"
  context_info+="⚠️ CLAUDE.md not found\n"
fi

if [ -f ".claude/memory.md" ]; then
  echo "[deep-work] ✅ found .claude/memory.md"
  context_info+="✅ .claude/memory.md available\n"
else
  echo "[deep-work] ⚠️  .claude/memory.md not found"
  context_info+="⚠️ .claude/memory.md not found\n"
fi

echo "[deep-work] checking recent commits..."
recent_commits=$(git log --oneline -10 main)
context_info+="📝 Recent commits:\n$recent_commits\n"

# Step 2: Planning phase
echo ""
echo "[deep-work] === Step 2: Planning with architectobot ==="

planning_prompt="You are starting work on GitHub issue #${issue} from ${repo}.

Working branch: $(git branch --show-current)
Issue URL: ${url}
Issue title: ${title}
Labels: ${labels:-(none)}

Context information:
${context_info}

--- Issue body ---
${body}
--- end issue body ---

Please analyze this issue and create a detailed implementation plan. Follow the workflow in CLAUDE.md and consider the existing codebase architecture. Break down the work into clear, manageable steps.

Focus on:
1. Understanding the requirements
2. Identifying affected components
3. Planning the implementation approach
4. Considering testing strategy
5. Potential edge cases or challenges

Output a structured plan that can guide the implementation phase."

echo "[deep-work] 🧠 calling architectobot for planning..."

# Call architectobot agent directly via claude with task flag
echo "$planning_prompt" | claude --task architectobot

echo ""
echo "[deep-work] 📋 Planning complete. Press Enter to continue to implementation..."
read -r

# Step 3: Implementation phase
echo ""
echo "[deep-work] === Step 3: Implementation with codecrusher ==="

implementation_prompt="Continue working on GitHub issue #${issue} from the planning phase.

Working branch: $(git branch --show-current)
Issue URL: ${url}
Issue title: ${title}

The architectobot has analyzed the requirements and created a plan. Now implement the solution:

--- Issue body ---
${body}
--- end issue body ---

Follow the implementation plan and:
1. Write clean, idiomatic code following existing patterns
2. Add appropriate tests for new functionality
3. Update documentation if needed
4. Ensure all changes are minimal and focused
5. Follow the project's coding conventions in CLAUDE.md

Keep the diff minimal and focused on the specific issue requirements."

echo "[deep-work] 💻 calling codecrusher for implementation..."

# Call codecrusher agent directly via claude with task flag
echo "$implementation_prompt" | claude --task codecrusher

echo ""
echo "[deep-work] ✅ Implementation complete. Press Enter to continue to quality checks..."
read -r

# Step 4 & 5: Quality checks
echo ""
echo "[deep-work] === Steps 4-5: Quality checks ==="
if ! run_quality_checks "implementation"; then
  echo "[deep-work] ❌ quality checks failed. Please fix issues and re-run."
  echo "[deep-work] to resume: pnpm deep-work continue $issue"
  exit 1
fi

# Step 6: Memory updates
echo ""
echo "[deep-work] === Step 6: Memory updates ==="
echo "[deep-work] 🧠 calling memory-keeper to update project knowledge..."

memory_prompt="I've just completed implementation work on GitHub issue #${issue}: \"${title}\".

Please update the project memory (.claude/memory.md) with any important learnings, patterns, gotchas, or insights from this work that would help future development on this project.

Focus on:
- New patterns or conventions established
- Technical challenges overcome
- Important gotchas or edge cases discovered
- Useful debugging techniques
- Architecture insights
- Testing approaches that worked well

Only add genuinely useful information that would help someone working on similar issues in the future."

# Call memory-keeper agent directly via claude with task flag
echo "$memory_prompt" | claude --task memory-keeper

# Step 7: Commit
echo ""
echo "[deep-work] === Step 7: Commit ==="

echo "[deep-work] staging changes for commit..."
git add .

# Generate commit message
commit_message="fix(#${issue}): ${title}

${body}

🤖 Generated with [Claude Code](https://claude.ai/code)

Co-Authored-By: Claude <noreply@anthropic.com>"

echo "[deep-work] committing changes..."
git commit -m "$(cat <<EOF
$commit_message
EOF
)"

echo "[deep-work] ✅ changes committed"

# Step 8: Merge main and resolve conflicts
echo ""
echo "[deep-work] === Step 8: Merge main & resolve conflicts ==="
echo "[deep-work] fetching latest main..."

# Fetch main from main worktree location
cd "$repo_root"
sync_upstream
cd "$worktree_dir"

echo "[deep-work] merging main into working branch..."
if ! git merge main; then
  echo "[deep-work] ⚠️  merge conflicts detected. Please resolve conflicts and continue."
  echo "[deep-work] after resolving conflicts, run: pnpm deep-work continue $issue"
  exit 1
fi

echo "[deep-work] ✅ main merged successfully"

# Step 9: Push and create draft PR
echo ""
echo "[deep-work] === Step 9: Push & create draft PR ==="

branch=$(git branch --show-current)
echo "[deep-work] pushing branch $branch to origin..."

# Push to origin (user's fork)
git push -u origin "$branch"

# Create draft PR
echo "[deep-work] creating draft PR..."
pr_body="## Summary

Closes #${issue}

${body}

## Changes Made

[Brief description of implementation approach]

## Testing

- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual testing completed
- [ ] Quality checks pass (typecheck, lint, format, build)

## Checklist

- [x] Code follows project conventions
- [x] Tests added for new functionality
- [x] Documentation updated if needed
- [x] Quality checks pass
- [x] Memory updated with learnings

🤖 Generated with [Claude Code](https://claude.ai/code)"

pr_url=$(gh pr create \
  --title "fix(#${issue}): ${title}" \
  --body "$pr_body" \
  --draft \
  --head "$(gh auth status 2>&1 | grep 'Logged in.*as' | sed 's/.*as //' | cut -d' ' -f1):$branch" \
  --base main \
  --repo "$repo")

echo "[deep-work] 📝 draft PR created: $pr_url"

# Step 10: Review cycle
echo ""
echo "[deep-work] === Step 10: Review cycle ==="
echo "[deep-work] 🔍 calling pr-reviewer for automated review..."

review_prompt="Please perform a thorough CodeRabbit-style review of this PR: $pr_url

This PR addresses issue #${issue}: \"${title}\"

Provide:
1. Walkthrough of changes
2. Change summary table
3. Per-file analysis
4. Inline comments with concrete suggestions
5. Identify any potential issues or improvements

After review, apply any approved suggestions, run quality checks, commit and push updates."

# Call pr-reviewer agent directly via claude with task flag
echo "$review_prompt" | claude --task pr-reviewer

# Step 11: Mark ready for review (user confirmation)
echo ""
echo "[deep-work] === Step 11: Ready for review ==="
echo "[deep-work] 🎉 Workflow complete! The PR has been created and auto-reviewed."
echo ""
echo "PR URL: $pr_url"
echo "Branch: $branch"
echo "Worktree: $worktree_dir"
echo ""
echo "Would you like to mark the PR as ready for review? (y/N)"
read -p "> " -r
if [[ $REPLY =~ ^[Yy]$ ]]; then
  echo "[deep-work] marking PR as ready for review..."
  gh pr ready "$pr_url"
  echo "[deep-work] ✅ PR marked as ready for review"
else
  echo "[deep-work] PR remains in draft. You can mark it ready later with:"
  echo "  gh pr ready $pr_url"
fi

# Step 12: Cleanup (user confirmation)
echo ""
echo "[deep-work] === Step 12: Cleanup ==="
echo ""
echo "The worktree for issue #$issue is at $worktree_dir (branch: $branch)."
echo ""
echo "Would you like me to clean up the worktree? This will:"
echo "- Remove the worktree directory"
echo "- Delete the local branch"
echo "- Keep the remote branch and PR intact"
echo ""
echo "Clean up worktree? (y/N)"
read -p "> " -r
if [[ $REPLY =~ ^[Yy]$ ]]; then
  cd "$repo_root"
  cleanup_worktree "$issue" "true"
  echo "[deep-work] ✅ worktree cleaned up"
else
  echo "[deep-work] worktree preserved. You can clean it up later with:"
  echo "  pnpm deep-work cleanup $issue"
fi

echo ""
echo "[deep-work] 🎊 Deep work session complete!"
echo "PR: $pr_url"
echo ""
echo "Next steps:"
echo "- Monitor PR for reviewer feedback"
echo "- Address any review comments"
echo "- Merge when approved"