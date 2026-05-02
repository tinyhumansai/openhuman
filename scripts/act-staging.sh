#!/usr/bin/env bash
# Run release-staging.yml (and the reusable build-desktop.yml it calls) under
# act, our local GitHub Actions runner. Reads secrets/vars from
# scripts/ci-secrets.json (gitignored), regenerates the dotenv-format
# .secrets / .vars files act consumes, and fakes a workflow_dispatch event
# from the staging branch.
#
# Usage:
#   bash scripts/act-staging.sh [extra act args]
# Examples:
#   bash scripts/act-staging.sh -j prepare-build       # only the bump+tag job
#   bash scripts/act-staging.sh --list                 # list jobs that would run
#   bash scripts/act-staging.sh -n                     # dry run
#
# Notes
# - The workflow's `Enforce main branch` step compares `github.ref` against
#   `refs/heads/main`; the event payload below sets that.
# - act maps `runs-on: macos-latest` / `windows-latest` to linux containers
#   by default. Real macOS notarization / Windows MSI signing cannot run here.
#   For local debugging, restrict to `-j prepare-build` or pair with a
#   matrix-platform filter via `--matrix settings.platform:ubuntu-22.04`.
# - `git push origin main` and tag pushes inside the container will hit the
#   real GitHub remote with the inherited token — every full run produces a
#   real `vX.Y.Z-staging` tag and a real bump commit on upstream `main`.
#   To avoid that, either pass `-n` for a dry run, or scope to a read-only
#   slice with `--list` / a job that has no side effects.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS_JSON="${ROOT}/scripts/ci-secrets.json"
SECRETS_FILE="${ROOT}/.secrets"
VARS_FILE="${ROOT}/.vars"
EVENT_FILE="${ROOT}/.github/act-event.json"
ACTRC_FILE="${ROOT}/.actrc"

if [ ! -f "$SECRETS_JSON" ]; then
  echo "Missing $SECRETS_JSON" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required (brew install jq)." >&2
  exit 1
fi

if ! command -v act >/dev/null 2>&1; then
  echo "act is required (brew install act)." >&2
  exit 1
fi

# act parses .secrets / .vars with a Go dotenv reader that supports
# double-quoted values with `\n` escapes — the only sane way to ship the
# PEM-formatted GitHub App private key. Use node so we can emit JSON
# strings ("...") that the parser will read back losslessly.
emit_dotenv() {
  local key="$1"
  local out="$2"
  node -e '
    const fs = require("fs");
    const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8"))[process.argv[2]] || {};
    const lines = Object.entries(data).map(([k, v]) => `${k}=${JSON.stringify(String(v))}`);
    fs.writeFileSync(process.argv[3], lines.join("\n") + "\n", { mode: 0o600 });
  ' "$SECRETS_JSON" "$key" "$out"
}

echo "[act-staging] regenerating $SECRETS_FILE"
emit_dotenv secrets "$SECRETS_FILE"
# act expects `GITHUB_TOKEN`; the JSON stores it under `GITHUB_TOKEN_` (or
# `XGH_TOKEN`) so the hostshell's `GITHUB_TOKEN` env doesn't clash with `gh`.
# Append a translated alias if either source key is present.
node -e '
  const fs = require("fs");
  const s = JSON.parse(fs.readFileSync(process.argv[1], "utf8")).secrets || {};
  const tok = s.GITHUB_TOKEN_ || s.XGH_TOKEN;
  if (tok) fs.appendFileSync(process.argv[2], `GITHUB_TOKEN=${JSON.stringify(String(tok))}\n`);
' "$SECRETS_JSON" "$SECRETS_FILE"
chmod 600 "$SECRETS_FILE"

echo "[act-staging] regenerating $VARS_FILE"
emit_dotenv vars "$VARS_FILE"
chmod 600 "$VARS_FILE"

echo "[act-staging] regenerating $ACTRC_FILE"
# Pinned in the script (not committed) because `.actrc` is in .gitignore;
# every developer's local act invocation goes through this script anyway.
cat > "$ACTRC_FILE" <<'ACTRC'
--container-architecture linux/amd64
-P ubuntu-22.04=catthehacker/ubuntu:act-22.04
-P ubuntu-latest=catthehacker/ubuntu:act-22.04
-P macos-latest=catthehacker/ubuntu:act-22.04
-P windows-latest=catthehacker/ubuntu:act-22.04
--pull=false
# Reuse cached action source under ~/.cache/act/ instead of re-cloning on
# every run. Required because act 0.2.87's go-git client always tries HTTP
# basic auth and gets 401 from github.com whether or not GITHUB_TOKEN is
# set (--token is not a CLI flag in 0.2.87). To refresh a cached action,
# delete the corresponding folder under ~/.cache/act/ and run with --pull.
--action-offline-mode
ACTRC

echo "[act-staging] regenerating $EVENT_FILE"
mkdir -p "$(dirname "$EVENT_FILE")"
# release-staging.yml's `Enforce main branch` step requires `github.ref ==
# refs/heads/main` — staging cuts and production both bump and tag from
# main. Set the dispatch ref accordingly.
cat > "$EVENT_FILE" <<'JSON'
{
  "ref": "refs/heads/main",
  "ref_name": "main",
  "ref_type": "branch",
  "repository": {
    "name": "openhuman",
    "full_name": "tinyhumansai/openhuman",
    "default_branch": "main"
  },
  "inputs": {}
}
JSON

# act uses GITHUB_TOKEN from the env / secret context to authenticate the
# go-git clones it performs for third-party actions (e.g.
# tibdex/github-app-token@v1). Prefer the local `gh` CLI token here — it's
# the user's OAuth token and has unrestricted public-repo read access. The
# fine-grained PAT in ci-secrets.json is scoped to this repo only and gets
# rejected with "Invalid username or token" on cross-repo action clones.
if command -v gh >/dev/null 2>&1; then
  GH_AUTH_TOKEN="$(gh auth token 2>/dev/null || true)"
  if [ -n "$GH_AUTH_TOKEN" ]; then
    export GITHUB_TOKEN="$GH_AUTH_TOKEN"
  fi
fi
if [ -z "${GITHUB_TOKEN:-}" ]; then
  echo "[act-staging] warning: no GITHUB_TOKEN available — third-party action clones may 401." >&2
fi

# act derives `github.repository` from the local checkout's parent dirs
# (so a fork like `senamakel/openhuman` becomes the value). Pin it to the
# upstream slug so steps that look up the GitHub App installation
# (`tibdex/github-app-token`) and that hit the GitHub API for the right
# repo (`gh release upload`, `gh api packages/...`) target the same repo
# CI sees in production.
exec act workflow_dispatch \
  -W "${ROOT}/.github/workflows/release-staging.yml" \
  --eventpath "$EVENT_FILE" \
  --secret-file "$SECRETS_FILE" \
  --var-file "$VARS_FILE" \
  --env GITHUB_REPOSITORY=tinyhumansai/openhuman \
  --env GITHUB_REPOSITORY_OWNER=tinyhumansai \
  "$@"
