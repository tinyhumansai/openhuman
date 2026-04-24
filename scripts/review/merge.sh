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
force=0
admin=0
auto=0
summary_llm="gemini"
shift
while [ $# -gt 0 ]; do
  case "$1" in
    --squash|--merge|--rebase) strategy="$1"; shift ;;
    --dry-run|-n) dry_run=1; shift ;;
    --force|-f) force=1; shift ;;
    --admin) admin=1; shift ;;
    --auto) auto=1; shift ;;
    --summary-llm) summary_llm="${2:?--summary-llm requires a value}"; shift 2 ;;
    --summary-llm=*) summary_llm="${1#*=}"; shift ;;
    *)
      echo "[review] unknown arg: $1 (expected --squash|--merge|--rebase|--dry-run|--force|--admin|--auto|--summary-llm)" >&2
      exit 1
      ;;
  esac
done

if [ "$admin" = "1" ] && [ "$auto" = "1" ]; then
  echo "[review] --admin and --auto are mutually exclusive" >&2
  exit 1
fi

repo=$(resolve_repo)

echo "[review] PR #$pr status on $repo:"
pr_status_json=$(gh pr view "$pr" -R "$repo" \
  --json state,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup)
jq '{state, mergeable, mergeStateStatus, reviewDecision,
     checks: [.statusCheckRollup[]? | {name: (.name // .context), status, conclusion}]}' \
  <<<"$pr_status_json"

# Merge gate: all required checks green + at least one maintainer approval.
# GitHub already folds both into mergeStateStatus:
#   CLEAN    — approved, required checks pass, mergeable
#   UNSTABLE — approved, required pass, non-required failing (OK to merge)
#   BLOCKED  — missing review, failing required check, or branch-protection block
#   DIRTY    — merge conflicts
# We require reviewDecision=APPROVED too, so a repo without branch protection
# (which would leave mergeStateStatus=CLEAN even without a review) still blocks.
ensure_merge_ready() {
  local state review merge_state
  state=$(jq -r '.state' <<<"$pr_status_json")
  review=$(jq -r '.reviewDecision // "NONE"' <<<"$pr_status_json")
  merge_state=$(jq -r '.mergeStateStatus' <<<"$pr_status_json")

  local ok=1
  if [ "$state" != "OPEN" ]; then
    echo "[review] ! PR state is $state (expected OPEN)" >&2
    ok=0
  fi
  case "$review" in
    APPROVED) ;;
    *)
      echo "[review] ! reviewDecision is $review (expected APPROVED — need a maintainer approval)" >&2
      ok=0
      ;;
  esac
  case "$merge_state" in
    CLEAN|UNSTABLE|HAS_HOOKS) ;;
    *)
      echo "[review] ! mergeStateStatus is $merge_state (expected CLEAN/UNSTABLE — required checks may still be pending or failing)" >&2
      ok=0
      ;;
  esac

  # Enumerate any required-looking checks that aren't SUCCESS/NEUTRAL/SKIPPED
  # for a clearer error. mergeStateStatus already covers this; this is just UX.
  local bad_checks
  bad_checks=$(jq -r '
      .statusCheckRollup[]?
      | select(
          (.conclusion // "") as $c
          | (.status // "") as $s
          | ($c | IN("SUCCESS","NEUTRAL","SKIPPED","")) as $okConc
          | ($s | IN("COMPLETED","")) as $okStatus
          | (($okConc and $okStatus) | not)
        )
      | "  - \((.name // .context)): status=\(.status // "?"), conclusion=\(.conclusion // "?")"
    ' <<<"$pr_status_json")
  if [ -n "$bad_checks" ]; then
    echo "[review] ! checks not green:" >&2
    printf '%s\n' "$bad_checks" >&2
    ok=0
  fi

  if [ "$ok" != "1" ]; then
    if [ "$force" = "1" ]; then
      echo "[review] --force: proceeding despite merge gate failures." >&2
      return 0
    fi
    echo "[review] refusing to merge. Re-run with --force to override." >&2
    exit 1
  fi
}

# Substring patterns (case-insensitive) matched against co-author name OR email.
# Override via REVIEW_BANNED_COAUTHOR_RE env var.
BANNED_RE="${REVIEW_BANNED_COAUTHOR_RE:-copilot|codex|cursor|claude|anthropic|openai|chatgpt|\[bot\]|noreply@github|users\.noreply\.github\.com}"

build_squash_body() {
  local pr="$1" repo="$2" summary_llm="$3" closing_issues="${4:-}"
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

  # Strip any stray closing-keyword lines the LLM or PR body may have
  # emitted — we'll append a canonical block below so GitHub sees one
  # `Closes #N` per linked issue (its regex only matches one ref per keyword,
  # so `Closes #1, #2` would only close #1).
  local summary_clean
  summary_clean=$(printf '%s\n' "$summary_body" \
    | grep -viE '^[[:space:]]*(close[sd]?|fix(e[sd])?|resolve[sd]?)[[:space:]]+(#|[A-Za-z0-9._-]+/[A-Za-z0-9._-]+#)[0-9]+' \
    || true)

  local closes_block=""
  if [ -n "$closing_issues" ]; then
    local n
    for n in $closing_issues; do
      closes_block+="Closes #${n}"$'\n'
    done
  fi

  {
    if [ -n "$summary_clean" ]; then
      printf '%s\n\n' "$summary_clean"
    fi
    if [ -n "$closes_block" ]; then
      printf '%s\n' "$closes_block"
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

  body=$(build_squash_body "$pr" "$repo" "$summary_llm" "$closing")
  echo "[review] squash commit message:"
  printf -- '----\n%s (#%s)\n\n%s\n----\n' "$title" "$pr" "$body"
  if [ "$dry_run" = "1" ]; then
    echo "[review] --dry-run: not merging."
    exit 0
  fi
  extra_flags=()
  if [ "$admin" = "1" ]; then
    echo "[review] --admin: bypassing local gate and using branch-protection override"
    extra_flags+=(--admin)
  elif [ "$auto" = "1" ]; then
    echo "[review] --auto: queueing merge once checks/approvals are satisfied"
    extra_flags+=(--auto)
  else
    ensure_merge_ready
  fi
  echo "[review] merging PR #$pr with --squash..."
  gh pr merge "$pr" -R "$repo" --squash --delete-branch \
    --subject "$title (#$pr)" \
    --body "$body" \
    "${extra_flags[@]}"
else
  if [ "$dry_run" = "1" ]; then
    echo "[review] --dry-run: $strategy does not rewrite the commit message; nothing to preview."
    exit 0
  fi
  extra_flags=()
  if [ "$admin" = "1" ]; then
    echo "[review] --admin: bypassing local gate and using branch-protection override"
    extra_flags+=(--admin)
  elif [ "$auto" = "1" ]; then
    echo "[review] --auto: queueing merge once checks/approvals are satisfied"
    extra_flags+=(--auto)
  else
    ensure_merge_ready
  fi
  echo "[review] merging PR #$pr with $strategy..."
  gh pr merge "$pr" -R "$repo" "$strategy" --delete-branch "${extra_flags[@]}"
fi
echo "[review] merged."
