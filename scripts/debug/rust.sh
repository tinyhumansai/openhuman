#!/usr/bin/env bash
# rust.sh [test-filter] [--verbose] [-- <cargo-test-args>…]
# Wraps scripts/test-rust-with-mock.sh.

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$here/lib.sh"

verbose=0
filter=""
passthrough=()
while [ $# -gt 0 ]; do
  case "$1" in
    --verbose) verbose=1; shift ;;
    --) shift; passthrough+=("$@"); break ;;
    -*) passthrough+=("$1"); shift ;;
    *)
      if [ -z "$filter" ]; then filter="$1"; else passthrough+=("$1"); fi
      shift ;;
  esac
done

repo_root="$(debug_repo_root)"
log_dir="$(debug_log_dir)"
log="$log_dir/rust-$(debug_timestamp).log"

cmd=(bash "$repo_root/scripts/test-rust-with-mock.sh")
if [ ${#passthrough[@]} -gt 0 ]; then
  cmd+=("${passthrough[@]}")
fi
if [ -n "$filter" ]; then
  cmd+=("$filter")
fi

echo "[debug:rust] log: $log"
echo "[debug:rust] cmd: ${cmd[*]}"
rc=0
debug_run "$log" "$verbose" -- "${cmd[@]}" || rc=$?

if [ "$verbose" != "1" ]; then
  debug_summarize_cargo "$log"
fi

if [ "$rc" != "0" ]; then
  echo
  echo "[debug:rust] FAILED (exit $rc) — full log: $log"
else
  echo "[debug:rust] OK — log: $log"
fi
exit "$rc"
