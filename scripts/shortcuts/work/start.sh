#!/usr/bin/env bash
# start.sh <issue-number> [extra-prompt] [--agent <tool>] [--no-checkout]
#
# Pick up a GitHub issue:
#   1. Sync `main` from upstream.
#   2. Create a working branch `<prefix>/<num>-<slug>` (slug from issue title).
#   3. Pull the issue (title/body/labels) via gh.
#   4. Hand off to the agent CLI with a prompt that includes the issue plus
#      repo conventions (CLAUDE.md / AGENTS.md pointers).
#
# --agent picks the CLI that drives the work. Default: claude.
# `--agent codex` uses `codex exec --dangerously-bypass-approvals-and-sandbox`
# and `--agent cursor` / `cursor-agent` use `cursor-agent --yolo`, so those
# sessions start in their equivalent "yolo" mode.
# A trailing positional <extra-prompt> is appended to the agent prompt.
# --no-checkout skips git sync/branch creation (use the current branch as-is).

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../review/lib.sh
source "$here/../review/lib.sh"

require git gh jq

if [ -z "${1:-}" ]; then
  echo "Usage: pnpm work <issue-number> [extra-prompt] [--agent <tool>] [--no-checkout]" >&2
  exit 1
fi
case "$1" in
  ''|*[!0-9]*)
    echo "[work] issue-number must be numeric, got: $1" >&2
    exit 1
    ;;
esac

issue="$1"
shift
agent="claude"
extra_prompt=""
do_checkout=1
while [ $# -gt 0 ]; do
  case "$1" in
    --agent) agent="${2:?--agent requires a value}"; shift 2 ;;
    --agent=*) agent="${1#*=}"; shift ;;
    --no-checkout) do_checkout=0; shift ;;
    *)
      if [ -n "$extra_prompt" ]; then
        echo "[work] unexpected extra arg: $1 (extra-prompt already set)" >&2
        exit 1
      fi
      extra_prompt="$1"; shift
      ;;
  esac
done

require "$agent"

# resolve_repo() lives in scripts/shortcuts/review/lib.sh; honour WORK_REPO override too.
repo="${WORK_REPO:-${REVIEW_REPO:-}}"
if [ -z "$repo" ]; then
  repo=$(REVIEW_REPO= resolve_repo)
fi
branch_prefix="${WORK_BRANCH_PREFIX:-issue}"

echo "[work] fetching issue #$issue from $repo..."
issue_json=$(gh issue view "$issue" -R "$repo" \
  --json number,title,body,labels,state,url,assignees)

state=$(jq -r '.state' <<<"$issue_json")
if [ "$state" != "OPEN" ]; then
  echo "[work] ! issue #$issue is $state — continuing anyway" >&2
fi

title=$(jq -r '.title' <<<"$issue_json")
body=$(jq -r '.body // ""' <<<"$issue_json")
url=$(jq -r '.url' <<<"$issue_json")
labels=$(jq -r '[.labels[].name] | join(", ")' <<<"$issue_json")

# Slug: lowercase, alnum + hyphens, max 40 chars, trimmed.
slug=$(printf '%s' "$title" \
  | tr '[:upper:]' '[:lower:]' \
  | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
  | cut -c1-40 \
  | sed -E 's/-+$//')
if [ -z "$slug" ]; then
  slug="work"
fi
branch="${branch_prefix}/${issue}-${slug}"

if [ "$do_checkout" = "1" ]; then
  echo "[work] syncing main..."
  git checkout main
  if git remote get-url upstream >/dev/null 2>&1; then
    git fetch upstream
    git merge --ff-only upstream/main || git merge upstream/main
  fi
  if git remote get-url origin >/dev/null 2>&1; then
    git pull --ff-only origin main
  fi
  git submodule update --init --recursive

  if git show-ref --verify --quiet "refs/heads/$branch"; then
    echo "[work] branch $branch already exists — checking it out and merging main"
    git checkout "$branch"
    if ! git merge main; then
      echo "[work] merge from main failed on branch $branch; resolve conflicts and re-run." >&2
      exit 1
    fi
  else
    echo "[work] creating branch $branch off main"
    git checkout -b "$branch"
  fi
else
  echo "[work] --no-checkout: staying on $(git branch --show-current)"
fi

current_branch=$(git branch --show-current)
labels_display="${labels:-(none)}"

template="$here/prompts/start.md"
if [ ! -f "$template" ]; then
  echo "[work] missing prompt template: $template" >&2
  exit 1
fi

# Use awk for substitution — handles multi-line values (issue body) cleanly.
# Escape backslashes and ampersands so gsub doesn't interpret them in the
# replacement text.
prompt=$(awk -v issue="$issue" -v repo="$repo" -v branch="$current_branch" \
             -v url="$url" -v title="$title" -v labels="$labels_display" \
             -v body="$body" '
  function esc(s) {
    gsub(/\\/, "\\\\", s); gsub(/&/, "\\\\&", s); return s
  }
  BEGIN {
    issue=esc(issue); repo=esc(repo); branch=esc(branch);
    url=esc(url); title=esc(title); labels=esc(labels); body=esc(body);
  }
  {
    gsub(/__ISSUE__/, issue);
    gsub(/__REPO__/, repo);
    gsub(/__BRANCH__/, branch);
    gsub(/__URL__/, url);
    gsub(/__TITLE__/, title);
    gsub(/__LABELS__/, labels);
    gsub(/__BODY__/, body);
    print
  }
' "$template")

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

# Additional instructions from the user
${extra_prompt}"
fi

echo "[work] handing off to ${agent} on branch ${current_branch}"
if [ "$agent" = "codex" ]; then
  codex exec --dangerously-bypass-approvals-and-sandbox "$prompt"
elif [ "$agent" = "cursor" ] || [ "$agent" = "cursor-agent" ]; then
  cursor-agent --yolo "$prompt"
else
  "$agent" "$prompt"
fi
