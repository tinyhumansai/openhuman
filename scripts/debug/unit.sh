#!/usr/bin/env bash
# unit.sh [pattern] [-t "<name>"] [--watch] [--verbose] [-- <vitest-args>…]
# Wraps `pnpm --filter openhuman-app test:unit`.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

verbose=0
watch=0
pattern=""
test_name=""
passthrough=()
while [ $# -gt 0 ]; do
  case "$1" in
    --verbose) verbose=1; shift ;;
    --watch) watch=1; shift ;;
    -t) test_name="${2:?-t requires a value}"; shift 2 ;;
    -t=*) test_name="${1#*=}"; shift ;;
    --) shift; passthrough+=("$@"); break ;;
    -*)
      passthrough+=("$1"); shift ;;
    *)
      if [ -z "$pattern" ]; then pattern="$1"; else passthrough+=("$1"); fi
      shift ;;
  esac
done

log_dir="$(debug_log_dir)"
log="$log_dir/unit-$(debug_timestamp).log"

repo_root="$(debug_repo_root)"
cd "$repo_root/app"

cmd=(pnpm exec vitest)
if [ "$watch" = "1" ]; then
  : # vitest default is watch
else
  cmd+=(run)
fi
cmd+=(--config test/vitest.config.ts)
if [ -n "$test_name" ]; then
  cmd+=(-t "$test_name")
fi
if [ -n "$pattern" ]; then
  cmd+=("$pattern")
fi
if [ ${#passthrough[@]} -gt 0 ]; then
  cmd+=("${passthrough[@]}")
fi

echo "[debug:unit] log: $log"
echo "[debug:unit] cmd: ${cmd[*]}"
rc=0
debug_run "$log" "$verbose" -- "${cmd[@]}" || rc=$?

if [ "$verbose" != "1" ]; then
  debug_summarize_vitest "$log"
fi

if [ "$rc" != "0" ]; then
  echo
  echo "[debug:unit] FAILED (exit $rc) — full log: $log"
else
  echo "[debug:unit] OK — log: $log"
fi
exit "$rc"
