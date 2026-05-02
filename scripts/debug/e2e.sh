#!/usr/bin/env bash
# e2e.sh <spec> [log-suffix] [--verbose]
# Wraps app/scripts/e2e-run-spec.sh with log capture + summary.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

verbose=0
spec=""
suffix=""
while [ $# -gt 0 ]; do
  case "$1" in
    --verbose) verbose=1; shift ;;
    -*)
      echo "[debug:e2e] unknown flag: $1" >&2; exit 1 ;;
    *)
      if [ -z "$spec" ]; then spec="$1"
      elif [ -z "$suffix" ]; then suffix="$1"
      else echo "[debug:e2e] unexpected arg: $1" >&2; exit 1
      fi
      shift ;;
  esac
done

if [ -z "$spec" ]; then
  echo "Usage: pnpm debug e2e <spec-path> [log-suffix] [--verbose]" >&2
  exit 1
fi

repo_root="$(debug_repo_root)"
log_dir="$(debug_log_dir)"
[ -n "$suffix" ] || suffix="$(basename "$spec" .spec.ts)"
log="$log_dir/e2e-${suffix}-$(debug_timestamp).log"

echo "[debug:e2e] spec: $spec"
echo "[debug:e2e] log:  $log"
rc=0
debug_run "$log" "$verbose" -- bash "$repo_root/app/scripts/e2e-run-spec.sh" "$spec" "$suffix" || rc=$?

if [ "$verbose" != "1" ]; then
  debug_summarize_wdio "$log"
fi

if [ "$rc" != "0" ]; then
  echo
  echo "[debug:e2e] FAILED (exit $rc) — full log: $log"
else
  echo "[debug:e2e] OK — log: $log"
fi
exit "$rc"
