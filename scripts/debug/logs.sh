#!/usr/bin/env bash
# logs.sh [list|last|<file>] [--head N | --tail N]

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

target="${1:-list}"
shift || true
mode=""
lines="200"
while [ $# -gt 0 ]; do
  case "$1" in
    --head) mode="head"; lines="${2:?--head requires N}"; shift 2 ;;
    --head=*) mode="head"; lines="${1#*=}"; shift ;;
    --tail) mode="tail"; lines="${2:?--tail requires N}"; shift 2 ;;
    --tail=*) mode="tail"; lines="${1#*=}"; shift ;;
    *) echo "[debug:logs] unknown arg: $1" >&2; exit 1 ;;
  esac
done

log_dir="$(debug_log_dir)"

if [ "$target" = "list" ]; then
  ls -1t "$log_dir" 2>/dev/null | head -n 50
  exit 0
fi

resolve_log() {
  local t="$1"
  if [ "$t" = "last" ]; then
    ls -1t "$log_dir" 2>/dev/null | head -n 1 | awk -v d="$log_dir" '{print d "/" $0}'
    return
  fi
  if [ -f "$t" ]; then echo "$t"; return; fi
  if [ -f "$log_dir/$t" ]; then echo "$log_dir/$t"; return; fi
  # Prefix match
  local match
  match=$(ls -1t "$log_dir" 2>/dev/null | grep -F "$t" | head -n 1 || true)
  if [ -n "$match" ]; then echo "$log_dir/$match"; return; fi
  echo ""
}

file="$(resolve_log "$target")"
if [ -z "$file" ] || [ ! -f "$file" ]; then
  echo "[debug:logs] no log matching: $target" >&2
  exit 1
fi

echo "[debug:logs] $file"
case "$mode" in
  head) head -n "$lines" "$file" ;;
  tail) tail -n "$lines" "$file" ;;
  "")
    if [ "$(wc -l <"$file")" -gt 400 ]; then
      echo "--- log is long; showing last 400 lines (use --head/--tail to override) ---"
      tail -n 400 "$file"
    else
      cat "$file"
    fi
    ;;
esac
