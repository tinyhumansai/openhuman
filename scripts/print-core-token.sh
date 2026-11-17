#!/usr/bin/env bash
#
# print-core-token.sh — print the active core RPC bearer token for the
# current deploy mode, so operators don't have to remember which side of the
# Tauri-vs-CLI / Docker-vs-binary split owns the value.
#
# Resolution order (matches crates/openhuman-core/src/core/auth.rs::init_rpc_token):
#   1. $OPENHUMAN_CORE_TOKEN if set and non-empty   (Tauri / Docker / cloud)
#   2. ${OPENHUMAN_WORKSPACE:-$HOME/.openhuman}/core.token   (standalone CLI)
#
# Usage:
#   scripts/print-core-token.sh           # print full token to stdout
#   scripts/print-core-token.sh --redact  # print first 8 hex chars + '…' only
#   scripts/print-core-token.sh --where   # print the source (env|file:path)
#                                          and exit without revealing the value
#
# Exit codes:
#   0 success
#   1 no token configured (neither env nor file)
#   2 file exists but is unreadable / empty
#
# This script is read-only and never logs the token to syslog or to debug
# files. When invoked from CI, prefer --redact + --where so logs stay safe.

set -euo pipefail

mode="full"
for arg in "$@"; do
  case "$arg" in
    --redact)
      mode="redact"
      ;;
    --where)
      mode="where"
      ;;
    -h|--help)
      sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "print-core-token: unknown argument '$arg'" >&2
      exit 64
      ;;
  esac
done

env_token="${OPENHUMAN_CORE_TOKEN:-}"
workspace_dir="${OPENHUMAN_WORKSPACE:-$HOME/.openhuman}"
file_path="$workspace_dir/core.token"

source="" # one of: env | file
token=""

if [ -n "$env_token" ]; then
  source="env"
  token="$env_token"
elif [ -f "$file_path" ]; then
  if [ ! -r "$file_path" ]; then
    echo "print-core-token: $file_path exists but is not readable by $USER" >&2
    exit 2
  fi
  token="$(tr -d '\n\r' < "$file_path" || true)"
  if [ -z "$token" ]; then
    echo "print-core-token: $file_path is empty" >&2
    exit 2
  fi
  source="file:$file_path"
else
  cat >&2 <<EOF
print-core-token: no core token configured.

Looked for:
  1. \$OPENHUMAN_CORE_TOKEN environment variable (used by Tauri shell, Docker,
     and any cloud deploy)
  2. $file_path (standalone CLI 'openhuman core run' writes this on first
     boot; override the directory with \$OPENHUMAN_WORKSPACE)

If you are running the dockerized core, set OPENHUMAN_CORE_TOKEN in your
.env (or the App Platform secrets UI) and bounce the container. See
docs/gitbooks/en/features/cloud-deploy.md for the full single-source-of-truth setup.
EOF
  exit 1
fi

case "$mode" in
  full)
    printf '%s\n' "$token"
    ;;
  redact)
    # Show enough to disambiguate two tokens without leaking the secret.
    head_chars="${token:0:8}"
    printf '%s…\n' "$head_chars"
    ;;
  where)
    printf '%s\n' "$source"
    ;;
esac
