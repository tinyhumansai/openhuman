#!/usr/bin/env bash
# Shared helpers for scripts/debug/*.sh. Source; do not execute.

set -euo pipefail

debug_repo_root() {
  (cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
}

debug_log_dir() {
  local root
  root="$(debug_repo_root)"
  mkdir -p "$root/target/debug-logs"
  echo "$root/target/debug-logs"
}

debug_timestamp() {
  date +%Y%m%d-%H%M%S
}

# Run a command, tee its combined output to a log file, return its exit code.
# Usage: debug_run <log_file> <verbose:0|1> -- <cmd> [args…]
debug_run() {
  local log="$1"; shift
  local verbose="$1"; shift
  if [ "${1:-}" = "--" ]; then shift; fi

  local rc=0
  if [ "$verbose" = "1" ]; then
    set +e
    "$@" 2>&1 | tee "$log"
    rc=${PIPESTATUS[0]}
    set -e
  else
    set +e
    "$@" >"$log" 2>&1
    rc=$?
    set -e
  fi
  return "$rc"
}

# Print a short summary + the failure block(s) from a Vitest log.
debug_summarize_vitest() {
  local log="$1"
  echo
  echo "--- summary ---"
  grep -E '^[[:space:]]*(Test Files|Tests|Duration|Start at)' "$log" | tail -n 20 || true
  if grep -qE '^[[:space:]]*FAIL ' "$log"; then
    echo
    echo "--- failures ---"
    grep -E '^[[:space:]]*FAIL ' "$log" || true
    echo
    echo "--- failure detail (first 200 lines after first FAIL) ---"
    awk '/^[[:space:]]*FAIL /{found=1} found{print; n++; if (n>=200) exit}' "$log"
  fi
}

# Print summary lines from a WDIO/Mocha run log.
debug_summarize_wdio() {
  local log="$1"
  echo
  echo "--- summary ---"
  grep -E '(passing|failing|pending|tests?, )' "$log" | tail -n 10 || true
  if grep -qE '^[[:space:]]*[0-9]+\)' "$log"; then
    echo
    echo "--- failure detail ---"
    awk '/^[[:space:]]*[0-9]+\)/{found=1} found{print}' "$log" | head -n 200
  fi
}

# Print summary + failure tails from a cargo-test log.
debug_summarize_cargo() {
  local log="$1"
  echo
  echo "--- summary ---"
  grep -E '^test result:' "$log" | tail -n 20 || true
  if grep -qE '^failures:' "$log"; then
    echo
    echo "--- failures ---"
    awk '/^failures:/{found=1} found{print}' "$log" | head -n 200
  fi
}
