#!/usr/bin/env bash
# Dispatcher for `pnpm deep-work <cmd> <args…>`.
# Commands: start, pick, continue, status, cleanup, list

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'EOF'
Usage: pnpm deep-work <command> [args...]

Full workflow automation for GitHub issues using worktrees and AI agents.

Commands:
  start <issue-number>           Start full workflow for an issue
  pick                          Smart issue selection + start workflow
  continue [issue-number]       Resume workflow from current step
  status                        Show all active worktrees and progress
  list                          List all worktrees
  cleanup <issue-number>        Clean up specific worktree (with confirmation)

Examples:
  pnpm deep-work start 1234     # Start working on issue #1234
  pnpm deep-work pick           # Let AI pick and start an issue
  pnpm deep-work continue       # Resume current work
  pnpm deep-work status         # Check progress on all issues
  pnpm deep-work cleanup 1234   # Clean up worktree for issue #1234
EOF
}

cmd="${1:-}"
if [ -z "$cmd" ] || [ "$cmd" = "-h" ] || [ "$cmd" = "--help" ]; then
  usage
  exit 0
fi

case "$cmd" in
  start)
    shift
    exec "$here/start.sh" "$@"
    ;;
  pick)
    shift
    exec "$here/pick.sh" "$@"
    ;;
  continue)
    shift
    exec "$here/continue.sh" "$@"
    ;;
  status)
    shift
    exec "$here/status.sh" "$@"
    ;;
  list)
    shift
    exec "$here/list.sh" "$@"
    ;;
  cleanup)
    shift
    exec "$here/cleanup.sh" "$@"
    ;;
  *)
    echo "[deep-work] unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac