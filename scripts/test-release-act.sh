#!/usr/bin/env bash
# Test the Release workflow locally using act.
#
# Defaults are safe:
# - Uses scripts/ci-secrets.example.json for secrets/vars.
# - Runs in dry-run mode unless --run is passed.
#
# For --run: set GitHub App credentials in scripts/ci-secrets.json:
# - XGITHUB_APP_ID
# - XGITHUB_APP_PRIVATE_KEY
# prepare-release uses those to mint a token for checkout/push.
# Do not put a bad GITHUB_TOKEN in ci-secrets.json — act uses it to clone action repos and an
# invalid PAT breaks even public clones.
#
# Usage:
#   ./scripts/test-release-act.sh
#   ./scripts/test-release-act.sh --run
#   ./scripts/test-release-act.sh --list
#   ./scripts/test-release-act.sh --job prepare-release
#   ./scripts/test-release-act.sh --release-type minor
#   ./scripts/test-release-act.sh --secrets-json scripts/ci-secrets.json --run
#   # Single macOS (Apple Silicon) build for signing — pass through to act --matrix:
#   ./scripts/test-release-act.sh --run --job build-artifacts \
#     --matrix 'settings.platform:macos-latest' --matrix 'settings.args:--target aarch64-apple-darwin'

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

WORKFLOW=".github/workflows/release.yml"
SECRETS_JSON="scripts/ci-secrets.json"
RELEASE_TYPE="patch"
RUN_MODE="dryrun"
JOB_NAME=""
MATRIX_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run)
      RUN_MODE="run"
      shift
      ;;
    --dryrun)
      RUN_MODE="dryrun"
      shift
      ;;
    --list)
      RUN_MODE="list"
      shift
      ;;
    --job)
      JOB_NAME="${2:-}"
      shift 2
      ;;
    --release-type)
      RELEASE_TYPE="${2:-patch}"
      shift 2
      ;;
    --secrets-json)
      SECRETS_JSON="${2:-}"
      shift 2
      ;;
    --matrix)
      MATRIX_ARGS+=(--matrix "${2:-}")
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ ! -f "$SECRETS_JSON" ]]; then
  echo "Secrets JSON not found: $SECRETS_JSON" >&2
  exit 1
fi

if ! command -v act >/dev/null 2>&1; then
  echo "act is required. Install with: brew install act" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required. Install with: brew install jq" >&2
  exit 1
fi

case "$RELEASE_TYPE" in
  major|minor|patch) ;;
  *)
    echo "--release-type must be one of: major, minor, patch" >&2
    exit 1
    ;;
esac

if [[ "$RUN_MODE" == "list" ]]; then
  act -W "$WORKFLOW" --list
  exit 0
fi

SECRETS_FILE="$(mktemp)"
VARS_FILE="$(mktemp)"
EVENT_JSON="$(mktemp)"
MERGED_SECRETS="$(mktemp)"
trap 'rm -f "$SECRETS_FILE" "$VARS_FILE" "$EVENT_JSON" "$MERGED_SECRETS"' EXIT

# Merge defaults: APPLE_APP_SPECIFIC_PASSWORD (APPLE_PASSWORD is a common alias).
# Do not put GITHUB_TOKEN in the act secret file — an invalid PAT breaks act's clone of public actions.
jq '
  .secrets |= (
    . + {
      APPLE_APP_SPECIFIC_PASSWORD: (
        if (.APPLE_APP_SPECIFIC_PASSWORD // "") | length > 0 then .APPLE_APP_SPECIFIC_PASSWORD
        else (.APPLE_PASSWORD // "") end
      )
    }
  )
' "$SECRETS_JSON" > "$MERGED_SECRETS"

# act --secret-file/--var-file expect dotenv format. Unquoted multiline values break the
# parser (PEM/private keys look like extra KEY= lines and trigger errors on '/' etc.).
jq -r '
def dotenv_escape:
  gsub("\""; "\\\"") | gsub("\r"; "\\r") | gsub("\n"; "\\n");
(.secrets // {}) | to_entries[] | select(.key != "GITHUB_TOKEN") | "\(.key)=\"\(.value | dotenv_escape)\""
' "$MERGED_SECRETS" > "$SECRETS_FILE"
jq -r '
def dotenv_escape:
  gsub("\""; "\\\"") | gsub("\r"; "\\r") | gsub("\n"; "\\n");
(.vars // {}) | to_entries[] | "\(.key)=\"\(.value | dotenv_escape)\""
' "$SECRETS_JSON" > "$VARS_FILE"

# Use real owner/repo from git so context.repo and tauri-action match your fork (not local/openhuman).
REPO_FULL="${GITHUB_REPOSITORY:-}"
if [[ -z "$REPO_FULL" ]]; then
  REPO_FULL="$(git remote get-url origin 2>/dev/null | sed -E 's#^git@github\.com:([^/]+)/([^/.]+)(\.git)?$#\1/\2#; s#^https://github\.com/([^/]+)/([^/.]+)(\.git)?$#\1/\2#')"
fi
if [[ -z "$REPO_FULL" || "$REPO_FULL" != */* ]]; then
  echo "Could not resolve GitHub owner/repo (set GITHUB_REPOSITORY or fix git remote origin)" >&2
  exit 1
fi
OWNER="${REPO_FULL%%/*}"
REPO_NAME="${REPO_FULL##*/}"

jq -n \
  --arg ref "refs/heads/main" \
  --arg rt "$RELEASE_TYPE" \
  --arg full "$REPO_FULL" \
  --arg owner "$OWNER" \
  --arg name "$REPO_NAME" \
  '{
    ref: $ref,
    inputs: { release_type: $rt },
    repository: {
      full_name: $full,
      default_branch: "main",
      name: $name,
      owner: { login: $owner }
    },
    sender: { login: "local-dev" }
  }' > "$EVENT_JSON"

echo "Workflow: $WORKFLOW"
echo "Secrets:  $SECRETS_JSON"
echo "Input:    release_type=$RELEASE_TYPE"
echo "Mode:     $RUN_MODE"
if [[ -n "$JOB_NAME" ]]; then
  echo "Job:      $JOB_NAME"
fi
if [[ ${#MATRIX_ARGS[@]} -gt 0 ]]; then
  echo "Matrix:   ${MATRIX_ARGS[*]}"
fi
echo

ACT_ARGS=(
  workflow_dispatch
  -W "$WORKFLOW"
  --eventpath "$EVENT_JSON"
  --secret-file "$SECRETS_FILE"
  --var-file "$VARS_FILE"
  --container-architecture linux/amd64
  -P ubuntu-latest=catthehacker/ubuntu:act-latest
  -P ubuntu-22.04=catthehacker/ubuntu:act-22.04
  -P macos-latest=-self-hosted
)

if [[ -n "$JOB_NAME" ]]; then
  ACT_ARGS+=(-j "$JOB_NAME")
fi

if [[ ${#MATRIX_ARGS[@]} -gt 0 ]]; then
  ACT_ARGS+=("${MATRIX_ARGS[@]}")
fi

if [[ "$RUN_MODE" == "dryrun" ]]; then
  echo "Dry-run only. Use --run to execute."
  act "${ACT_ARGS[@]}" -n
else
  act "${ACT_ARGS[@]}"
fi
