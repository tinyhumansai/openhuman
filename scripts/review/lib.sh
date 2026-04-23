#!/usr/bin/env bash
# Shared helpers for scripts/review/*.sh
# Source this file; do not execute directly.

set -euo pipefail

# Repo that hosts the PR. Override with REVIEW_REPO=owner/name if needed;
# otherwise we derive it from the `upstream` remote, falling back to `origin`.
resolve_repo() {
  if [ -n "${REVIEW_REPO:-}" ]; then
    echo "$REVIEW_REPO"
    return
  fi
  local url
  url=$(git remote get-url upstream 2>/dev/null || git remote get-url origin)
  # Accept git@github.com:owner/name(.git) and https://github.com/owner/name(.git)
  echo "$url" \
    | sed -E 's#^git@github\.com:##; s#^https?://github\.com/##; s#\.git$##'
}

require() {
  local bin
  for bin in "$@"; do
    command -v "$bin" >/dev/null 2>&1 || {
      echo "[review] missing required tool: $bin" >&2
      exit 1
    }
  done
}

# Summarize free-form text via a local LLM CLI (expects `-p <prompt>`).
# Usage: summarize_text <tool> <input>
# Tools used here: gemini (default for summaries), claude, or any CLI that
# accepts `-p "<prompt>"` and prints the response to stdout.
# Special value `none` echoes input unchanged.
summarize_text() {
  local tool="$1"
  local input="$2"
  if [ "$tool" = "none" ] || [ "$tool" = "raw" ]; then
    printf '%s' "$input"
    return
  fi
  require "$tool"
  local prompt
  prompt=$(cat <<'EOF'
You are writing the body of a squash-merge commit.
Summarize the PR changes below into 3-6 short bullet points.
Rules:
- Start each bullet with "- " and use imperative mood ("Add…", "Fix…", "Rename…").
- One line per bullet, under ~100 chars.
- No headers, no code fences, no sign-offs, no Co-authored-by lines.
- Do not include the PR number or title.
- Output only the bullets, nothing else.

PR content:
---
EOF
)
  "$tool" -p "${prompt}
${input}
---"
}

require_pr_number() {
  if [ -z "${1:-}" ]; then
    echo "Usage: $(basename "$0") <pr-number>" >&2
    exit 1
  fi
  case "$1" in
    ''|*[!0-9]*)
      echo "[review] pr-number must be numeric, got: $1" >&2
      exit 1
      ;;
  esac
}

# Fetch PR head into local branch pr/<num>, merge main in, wire upstream +
# pushRemote so `git push` lands on the contributor's fork.
sync_pr() {
  local pr="$1"
  local repo
  repo=$(resolve_repo)

  echo "[review] syncing main from upstream..."
  git checkout main
  git pull origin main
  git fetch upstream
  git merge upstream/main
  git submodule update --init --recursive

  local info head_repo head_branch local_branch
  info=$(gh pr view "$pr" -R "$repo" \
    --json headRefName,headRepository,headRepositoryOwner)
  head_repo=$(echo "$info" | jq -r '.headRepositoryOwner.login + "/" + .headRepository.name')
  head_branch=$(echo "$info" | jq -r '.headRefName')
  local_branch="pr/$pr"

  echo "[review] PR #$pr -> $head_repo:$head_branch (local: $local_branch)"

  git fetch "https://github.com/${head_repo}.git" \
    "+${head_branch}:${local_branch}"
  git checkout "$local_branch"

  echo "[review] merging main into $local_branch (conflicts will not abort)..."
  git merge main || echo "[review] ! conflicts detected in PR #$pr, continuing."

  # Prefer an existing SSH remote pointing at this fork to avoid https auth prompts.
  local remote_name="remote-$pr"
  local existing_ssh
  existing_ssh=$(git remote -v \
    | awk -v repo="$head_repo" '$2 ~ ("[:/]" repo "(\\.git)?$") && $3 == "(fetch)" {print $1; exit}')
  if [ -n "$existing_ssh" ]; then
    remote_name="$existing_ssh"
    echo "[review] reusing remote '$remote_name' -> $(git remote get-url "$remote_name")"
  else
    local remote_url="https://github.com/${head_repo}.git"
    git remote add "$remote_name" "$remote_url" 2>/dev/null \
      || git remote set-url "$remote_name" "$remote_url"
  fi

  git fetch "$remote_name" \
    "+refs/heads/${head_branch}:refs/remotes/${remote_name}/${head_branch}"

  git branch --set-upstream-to="$remote_name/$head_branch" "$local_branch"
  git config "branch.${local_branch}.pushRemote" "$remote_name"
  git config "branch.${local_branch}.merge" "refs/heads/${head_branch}"

  echo "[review] upstream + pushRemote set to $remote_name/$head_branch"

  # Export for callers.
  REVIEW_PR="$pr"
  REVIEW_REPO_RESOLVED="$repo"
  REVIEW_LOCAL_BRANCH="$local_branch"
  REVIEW_HEAD_REPO="$head_repo"
  REVIEW_HEAD_BRANCH="$head_branch"
}
