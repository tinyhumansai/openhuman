#!/usr/bin/env bash
# Dispatcher for `pnpm debug <cmd> <args…>`.
# Agent-friendly wrappers around the project's test/run scripts.
# Commands: unit | e2e | rust | logs

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'EOF'
Usage: pnpm debug <command> [args]

Commands:
  unit  [pattern] [-t "<name>"] [--watch] [--verbose]
        Run Vitest. Full log goes to target/debug-logs/unit-<ts>.log;
        stdout shows only summary + failure blocks unless --verbose.
  e2e   <spec> [log-suffix] [--verbose]
        Run a single WDIO spec via app/scripts/e2e-run-spec.sh.
        Full log goes to target/debug-logs/e2e-<suffix>-<ts>.log.
  rust  [test-filter] [--verbose]
        Run cargo tests with the mock backend (test-rust-with-mock.sh).
        Full log goes to target/debug-logs/rust-<ts>.log.
  logs  [list|<run-id>|last] [--head N | --tail N]
        Inspect saved debug-log files. `last` shows the most recent.

Flags common to runners:
  --verbose   Stream full output to stdout in addition to the log file.

Exit code = the underlying tool's exit code.
EOF
}

cmd="${1:-}"
if [ -z "$cmd" ] || [ "$cmd" = "-h" ] || [ "$cmd" = "--help" ]; then
  usage
  exit 0
fi
shift

case "$cmd" in
  unit|e2e|rust|logs)
    exec "$here/${cmd}.sh" "$@"
    ;;
  *)
    echo "[debug] unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac
