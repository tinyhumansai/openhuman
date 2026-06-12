#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release/upload-android-to-play.sh [--ref <git-ref>] [--track <track>] [--status <status>] [--no-watch]

Triggers the Android Build and Publish GitHub Actions workflow. Signing and
Google Play upload happen in CI using repository/environment secrets.

Options:
  --ref <git-ref>     Git ref/SHA to build. Defaults to the current branch.
  --track <track>     Play track: internal, alpha, beta, production. Default: internal.
  --status <status>   Play status: completed, draft, inProgress, halted. Default: completed.
  --no-watch          Do not wait for the GitHub Actions run.
  -h, --help          Show this help.
USAGE
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

ref="$(git branch --show-current 2>/dev/null || true)"
if [ -z "$ref" ]; then
  ref="$(git rev-parse HEAD)"
fi
track="${ANDROID_PLAY_TRACK:-internal}"
status="${ANDROID_PLAY_STATUS:-completed}"
watch=1

while [ "$#" -gt 0 ]; do
  case "$1" in
    --ref)
      ref="${2:-}"
      shift 2
      ;;
    --track)
      track="${2:-}"
      shift 2
      ;;
    --status)
      status="${2:-}"
      shift 2
      ;;
    --no-watch)
      watch=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$ref" =~ ^[0-9a-fA-F]{7,40}$ ]] && [ "$watch" -eq 1 ]; then
  echo "[android-play] ref is a SHA; skipping automatic watch because gh run list --branch requires a branch name."
  watch=0
fi

case "$track" in
  internal|alpha|beta|production) ;;
  *)
    echo "Invalid --track '$track'. Expected internal, alpha, beta, or production." >&2
    exit 2
    ;;
esac

case "$status" in
  completed|draft|inProgress|halted) ;;
  *)
    echo "Invalid --status '$status'. Expected completed, draft, inProgress, or halted." >&2
    exit 2
    ;;
esac

if ! command -v gh >/dev/null 2>&1; then
  echo "GitHub CLI is required. Install gh and run: gh auth login" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "GitHub CLI is not authenticated. Run: gh auth login" >&2
  exit 1
fi

echo "[android-play] triggering workflow for ref=$ref track=$track status=$status"
gh workflow run android-compile.yml \
  --ref "$ref" \
  -f build_ref="$ref" \
  -f publish_to_play=true \
  -f play_track="$track" \
  -f play_status="$status"

if [ "$watch" -eq 1 ]; then
  echo "[android-play] waiting for workflow run to appear..."
  sleep 8
  run_id="$(
    gh run list \
      --workflow android-compile.yml \
      --branch "$ref" \
      --limit 1 \
      --json databaseId \
      --jq '.[0].databaseId // empty'
  )"
  if [ -z "$run_id" ]; then
    echo "[android-play] workflow was triggered, but no run was found yet. Check Actions manually." >&2
    exit 0
  fi
  gh run watch "$run_id" --exit-status
fi
