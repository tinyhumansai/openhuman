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
# A trailing positional <extra-prompt> is appended to the agent prompt.
# --no-checkout skips git sync/branch creation (use the current branch as-is).

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../.." && pwd)"
# shellcheck source=../review/lib.sh
source "$repo_root/scripts/review/lib.sh"

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

# resolve_repo() lives in scripts/review/lib.sh; honour WORK_REPO override too.
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
  git pull --ff-only origin main || true
  git submodule update --init --recursive

  if git show-ref --verify --quiet "refs/heads/$branch"; then
    echo "[work] branch $branch already exists — checking it out and merging main"
    git checkout "$branch"
    git merge main || echo "[work] ! conflicts merging main, continuing."
  else
    echo "[work] creating branch $branch off main"
    git checkout -b "$branch"
  fi
else
  echo "[work] --no-checkout: staying on $(git branch --show-current)"
fi

current_branch=$(git branch --show-current)

prompt="You are picking up GitHub issue #${issue} on ${repo}.

Working branch: ${current_branch}
Issue URL: ${url}
Issue title: ${title}
Labels: ${labels:-(none)}

--- Issue body ---
${body}
--- end issue body ---

Follow the workflow in CLAUDE.md and AGENTS.md. Plan the change against the
existing domains, implement it, add tests, and keep the diff minimal. When the
implementation is ready, commit on this branch with a message that references
#${issue}, push, and open a PR targeting main using the repo's PR template. Do
not merge."

if [ -n "$extra_prompt" ]; then
  prompt="${prompt}

Additional instructions from the user:
${extra_prompt}"
fi

echo "[work] handing off to ${agent} on branch ${current_branch}"
"$agent" "$prompt"
