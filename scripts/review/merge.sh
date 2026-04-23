#!/usr/bin/env bash
# merge.sh <pr-number> [--squash|--merge|--rebase] [--dry-run] [--summary-llm <tool>]
# Merge a PR via gh. Defaults to --squash.
#
# For --squash we rewrite the commit body:
#   - summarize the PR body + commit messages with the summary LLM
#     (default: gemini; use `none` to skip and keep the raw PR body)
#   - drop any Co-authored-by lines mentioning copilot / codex / cursor / claude
#   - add the current `git config user.name <user.email>` as a co-author
# --merge and --rebase keep the original commits as-is.
#
# --dry-run prints the squash subject + body that would be used and exits
# without calling `gh pr merge`. Ignored for --merge / --rebase.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

require git gh jq
require_pr_number "${1:-}"

pr="$1"
strategy="--squash"
dry_run=0
summary_llm="gemini"
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --squash|--merge|--rebase) strategy="$1"; shift ;;
    --dry-run|-n) dry_run=1; shift ;;
    --summary-llm) summary_llm="${2:?--summary-llm requires a value}"; shift 2 ;;
    --summary-llm=*) summary_llm="${1#*=}"; shift ;;
    *)
      echo "[review] unknown arg: $1 (expected --squash|--merge|--rebase|--dry-run|--summary-llm)" >&2
      exit 1
      ;;
  esac
done

repo=$(resolve_repo)

echo "[review] PR #$pr status on $repo:"
gh pr view "$pr" -R "$repo" \
  --json state,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup \
  | jq '{state, mergeable, mergeStateStatus, reviewDecision,
         checks: [.statusCheckRollup[]? | {name: (.name // .context), status, conclusion}]}'

# Substring patterns (case-insensitive) matched against co-author name OR email.
# Override via REVIEW_BANNED_COAUTHOR_RE env var.
BANNED_RE="${REVIEW_BANNED_COAUTHOR_RE:-copilot|codex|cursor|claude|anthropic|openai|chatgpt|\[bot\]|noreply@github|users\.noreply\.github\.com}"

build_squash_body() {
  local pr="$1" repo="$2" summary_llm="$3"
  local data body title me_name me_email
  data=$(gh pr view "$pr" -R "$repo" --json title,body,commits)
  title=$(jq -r '.title' <<<"$data")
  body=$(jq -r '.body // ""' <<<"$data")

  me_name=$(git config --get user.name || true)
  me_email=$(git config --get user.email || true)
  if [ -z "$me_name" ] || [ -z "$me_email" ]; then
    echo "[review] git config user.name/user.email not set; cannot add self as co-author" >&2
    exit 1
  fi

  # Strip any existing Co-authored-by trailers from the PR body.
  local body_clean
  body_clean=$(printf '%s\n' "$body" | grep -viE '^co-authored-by:' || true)
  # Trim trailing blank lines.
  body_clean=$(printf '%s\n' "$body_clean" | awk 'NF {p=1} p {lines[NR]=$0; last=NR} END {for (i=1;i<=last;i++) print lines[i]}')

  # Build input for the summary LLM: title + PR body + commit list.
  local summary_input
  summary_input=$(jq -r '
      "Title: " + .title + "\n\n" +
      "PR body:\n" + (.body // "(empty)") + "\n\n" +
      "Commits:\n" +
      ((.commits // [])
        | map("- " + .messageHeadline
              + (if (.messageBody // "") != ""
                 then "\n  " + ((.messageBody) | gsub("\n"; "\n  "))
                 else "" end))
        | join("\n"))
    ' <<<"$data")

  local summary_body
  if [ "$summary_llm" = "none" ] || [ "$summary_llm" = "raw" ]; then
    summary_body="$body_clean"
  else
    echo "[review] summarizing with ${summary_llm}..." >&2
    summary_body=$(summarize_text "$summary_llm" "$summary_input")
    if [ -z "$summary_body" ]; then
      echo "[review] ! summary LLM returned empty output; falling back to PR body" >&2
      summary_body="$body_clean"
    fi
  fi

  # Collect co-authors from commit authors + Co-authored-by trailers, then
  # filter. tolower()-based match is portable (BSD awk has no IGNORECASE).
  local coauthors
  coauthors=$(jq -r '
      .commits[]
      | (
          (.authors[]? | "\(.name // "")\t\(.email // "")"),
          (.messageBody // "" | split("\n")[]
            | select(test("^[Cc]o-authored-by:"))
            | sub("^[Cc]o-authored-by:\\s*"; "")
            | capture("^(?<n>.+?)\\s*<(?<e>[^>]+)>\\s*$")?
            | "\(.n)\t\(.e)"
          )
        )
    ' <<<"$data" \
    | awk -F'\t' -v me="$me_email" -v banned="$BANNED_RE" '
        NF < 2 { next }
        $1 == "" || $2 == "" { next }
        tolower($2) == tolower(me) { next }
        {
          nl = tolower($1); el = tolower($2);
          if (nl ~ banned || el ~ banned) next;
          key = el;
          if (!(key in seen)) {
            seen[key] = 1
            printf "Co-authored-by: %s <%s>\n", $1, $2
          }
        }
      ')

  {
    if [ -n "$summary_body" ]; then
      printf '%s\n\n' "$summary_body"
    fi
    if [ -n "$coauthors" ]; then
      printf '%s\n' "$coauthors"
    fi
    printf 'Co-authored-by: %s <%s>\n' "$me_name" "$me_email"
  }
  : "$title"  # reserved for future subject overrides
}

if [ "$strategy" = "--squash" ]; then
  title=$(gh pr view "$pr" -R "$repo" --json title -q .title)

  # Append any linked "Closes #N" issues that aren't already referenced in the
  # title (skip issue numbers already mentioned as #N).
  closing=$(gh pr view "$pr" -R "$repo" \
    --json closingIssuesReferences \
    --jq '.closingIssuesReferences[].number' 2>/dev/null || true)
  missing=()
  for n in $closing; do
    if ! grep -qE "#${n}([^0-9]|$)" <<<"$title"; then
      missing+=("#${n}")
    fi
  done
  if [ ${#missing[@]} -gt 0 ]; then
    joined=$(printf ', %s' "${missing[@]}")
    joined=${joined:2}
    title="${title} (closes ${joined})"
  fi

  body=$(build_squash_body "$pr" "$repo" "$summary_llm")
  echo "[review] squash commit message:"
  printf -- '----\n%s (#%s)\n\n%s\n----\n' "$title" "$pr" "$body"
  if [ "$dry_run" = "1" ]; then
    echo "[review] --dry-run: not merging."
    exit 0
  fi
  echo "[review] merging PR #$pr with --squash..."
  gh pr merge "$pr" -R "$repo" --squash --delete-branch \
    --subject "$title (#$pr)" \
    --body "$body"
else
  if [ "$dry_run" = "1" ]; then
    echo "[review] --dry-run: $strategy does not rewrite the commit message; nothing to preview."
    exit 0
  fi
  echo "[review] merging PR #$pr with $strategy..."
  gh pr merge "$pr" -R "$repo" "$strategy" --delete-branch
fi
echo "[review] merged."
