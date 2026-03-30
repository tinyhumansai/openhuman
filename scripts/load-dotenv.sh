#!/usr/bin/env bash
# Load .env file into environment variables.
# Usage:
#   source scripts/load-dotenv.sh [path/to/.env]
#   eval "$(scripts/load-dotenv.sh [path/to/.env])"
# Default path: .env (project root when run from repo root)

set -e
FILE="${1:-.env}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RESOLVED="${1:+$1}"
RESOLVED="${RESOLVED:-$ROOT_DIR/.env}"

if [[ ! -f "$RESOLVED" ]]; then
  echo "File not found: $RESOLVED" >&2
  exit 1
fi

exports=()
while IFS= read -r line || [[ -n "$line" ]]; do
  line="${line%%#*}"
  line="${line#"${line%%[![:space:]]*}"}"
  line="${line%"${line##*[![:space:]]}"}"
  [[ -z "$line" ]] && continue
  if [[ "$line" == export\ * ]]; then
    line="${line#export }"
  fi
  if [[ "$line" == *"="* ]]; then
    key="${line%%=*}"
    key="${key%"${key##*[![:space:]]}"}"
    value="${line#*=}"
    value="${value#\"}"
    value="${value%\"}"
    value="${value#\'}"
    value="${value%\'}"
    [[ -n "$key" ]] && exports+=("$(printf 'export %s=%q' "$key" "$value")")
  fi
done < "$RESOLVED"

if [[ ${#exports[@]} -eq 0 ]]; then
  joined=""
else
  joined=$(printf '%s\n' "${exports[@]}")
fi

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  echo "$joined"
else
  eval "$joined"
fi
