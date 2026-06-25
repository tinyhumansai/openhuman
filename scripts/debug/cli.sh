#!/usr/bin/env bash
# Dispatcher for `pnpm debug <cmd> <args…>`.
# Agent-friendly wrappers around the project's test/run scripts.
# Commands: unit | e2e | rust | logs | harness-cache-audit

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
  harness-cache-audit [options]
        Run live harness turns over JSON-RPC and summarize transcript token/cache deltas.
  agent-prepare-context-audit [options]
        Live-audit the agent_prepare_context tool: force it per query, print the
        returned context bundle (incl. recommended_skills), scout thoughts,
        gathering tools used, and tokens/cache/cost. Seeds a prior-chat thread
        with a canary fact and adds a transcript-recall case to prove the scout
        searches past chats (--no-seed-transcript to skip).
  goals-live [options]
        Live-test the memory_goals flow (list/add/edit/delete + reflect enrichment),
        printing the goals_agent's thoughts, tool calls, token usage and cost.

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
  harness-cache-audit)
    exec node "$here/harness-cache-audit.mjs" "$@"
    ;;
  agent-prepare-context-audit)
    exec node "$here/agent-prepare-context-audit.mjs" "$@"
    ;;
  goals-live)
    exec node "$here/goals-live.mjs" "$@"
    ;;
  *)
    echo "[debug] unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac
