#!/usr/bin/env bash
# Test the package-and-publish workflow locally using `act`.
#
# Prerequisites:
#   brew install act jq
#
# Setup:
#   cp scripts/ci-secrets.example.json scripts/ci-secrets.json
#   # Edit scripts/ci-secrets.json with your real values
#
# Usage:
#   ./scripts/test-ci-local.sh              # Run full workflow via act
#   ./scripts/test-ci-local.sh --manual     # Run build steps natively on macOS (recommended)
#   ./scripts/test-ci-local.sh --list       # List available jobs
#   ./scripts/test-ci-local.sh --dryrun     # Dry-run (show what would execute)

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# ── Configuration ─────────────────────────────────────────────────────────────

WORKFLOW=".github/workflows/package-and-publish.yml"
SECRETS_JSON="scripts/ci-secrets.json"
EVENT_JSON="scripts/ci-event.json"

if [[ ! -f "$SECRETS_JSON" ]]; then
    echo "ERROR: $SECRETS_JSON not found."
    echo ""
    echo "Create it from the example:"
    echo "  cp scripts/ci-secrets.example.json scripts/ci-secrets.json"
    echo "  # then fill in your values"
    exit 1
fi

# ── Generate event payload with current HEAD ──────────────────────────────────

CURRENT_REF=$(git rev-parse HEAD)
cat > "$EVENT_JSON" <<EOF
{
  "ref": "refs/heads/develop",
  "before": "0000000000000000000000000000000000000000",
  "after": "$CURRENT_REF",
  "repository": {
    "full_name": "vezuresdotxyz/openhuman-frontend-runner",
    "default_branch": "main",
    "name": "openhuman-frontend-runner",
    "owner": { "login": "vezuresdotxyz" }
  },
  "head_commit": {
    "id": "$CURRENT_REF",
    "message": "local test build"
  },
  "sender": { "login": "local-dev" }
}
EOF

# ── Convert JSON to act-compatible KEY=VALUE files ────────────────────────────

SECRETS_FILE=$(mktemp)
VARS_FILE=$(mktemp)
trap 'rm -f "$SECRETS_FILE" "$VARS_FILE"' EXIT

# Extract "secrets" object → KEY=VALUE (quoted/escaped for act dotenv parsing)
jq -r '
def dotenv_escape:
  gsub("\""; "\\\"") | gsub("\r"; "\\r") | gsub("\n"; "\\n");
(.secrets // {}) | to_entries[] | "\(.key)=\"\(.value | dotenv_escape)\""
' "$SECRETS_JSON" > "$SECRETS_FILE"

# Extract "vars" object → KEY=VALUE
jq -r '
def dotenv_escape:
  gsub("\""; "\\\"") | gsub("\r"; "\\r") | gsub("\n"; "\\n");
(.vars // {}) | to_entries[] | "\(.key)=\"\(.value | dotenv_escape)\""
' "$SECRETS_JSON" > "$VARS_FILE"

echo "Loaded $(wc -l < "$SECRETS_FILE" | tr -d ' ') secrets and $(wc -l < "$VARS_FILE" | tr -d ' ') vars from $SECRETS_JSON"

# ── Common act arguments ──────────────────────────────────────────────────────

ACT_ARGS=(
    -W "$WORKFLOW"
    --secret-file "$SECRETS_FILE"
    --var-file "$VARS_FILE"
    --eventpath "$EVENT_JSON"
    -P ubuntu-latest=catthehacker/ubuntu:act-latest
    -P macos-latest=-self-hosted
)

# ── Handle CLI flags ──────────────────────────────────────────────────────────

if [[ "${1:-}" == "--list" ]]; then
    echo "Available jobs in $WORKFLOW:"
    act -W "$WORKFLOW" --list
    exit 0
fi

if [[ "${1:-}" == "--dryrun" ]]; then
    echo "Dry-run of workflow:"
    act push "${ACT_ARGS[@]}" -n
    exit 0
fi

# ── Manual macOS-native build (recommended) ───────────────────────────────────

if [[ "${1:-}" == "--manual" ]]; then
    echo "=== Running build steps manually on macOS host ==="
    echo ""

    # Export VITE_* vars from the JSON so the frontend build picks them up
    eval "$(jq -r '
      (.secrets // {}) + (.vars // {})
      | to_entries[]
      | select(.key | startswith("VITE_"))
      | "export \(.key)=\(.value | @sh)"
    ' "$SECRETS_JSON")"

    # Step 1: Ensure OpenSSL is installed
    echo ">>> Step 1: Ensure OpenSSL is installed"
    brew install openssl@3 2>/dev/null || true

    # Step 2: Install Node dependencies
    echo ">>> Step 2: Install Node dependencies"
    yarn install --frozen-lockfile

    # Step 3: Install skills dependencies and build
    echo ">>> Step 3: Build skills"
    (cd skills && yarn install --frozen-lockfile && yarn build)

    # Step 4: Build frontend
    echo ">>> Step 4: Build frontend"
    NODE_ENV=production yarn build

    # Step 5: Build Tauri (aarch64)
    echo ">>> Step 5: Build Tauri app (aarch64-apple-darwin)"
    yarn tauri build --target aarch64-apple-darwin

    echo ""
    echo "=== Build complete ==="
    echo "Check src-tauri/target/aarch64-apple-darwin/release/bundle/ for output"
    exit 0
fi

# ── Default: run full workflow with act ────────────────────────────────────────
#
# We run the full workflow (not -j single-job) so act executes the dependency
# chain: get-version → check-version → create-release (skipped) → package-tauri.
# Using -j package-tauri alone fails because act can't resolve outputs from
# skipped `needs` jobs.

echo "=== Testing package-and-publish workflow locally ==="
echo ""
echo "Workflow: $WORKFLOW"
echo "Event:    push to develop (from $EVENT_JSON)"
echo ""
echo "NOTE: act uses Docker containers — macOS-specific steps won't work."
echo "For a native macOS build, use: $0 --manual"
echo ""

act push "${ACT_ARGS[@]}" --verbose
