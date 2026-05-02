#!/usr/bin/env bash
# Dispatcher for `pnpm work <cmd> <args…>`.
# Commands: start (default)

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat <<'EOF'
Usage: pnpm work <issue-number> [extra-prompt] [--agent <tool>] [--no-checkout]
       pnpm work start <issue-number> [extra-prompt] [--agent <tool>] [--no-checkout]

Pick up a GitHub issue, create a working branch off main, and hand it to an
LLM CLI to start implementing.

Args:
  <issue-number>                 The GitHub issue to work on.
  [extra-prompt]                 Optional free-form text appended verbatim to
                                 the agent prompt.

Flags:
  --agent <tool>                 Agent CLI to drive (default: claude). The
                                 prompt is passed as a single positional
                                 argument: \`<tool> "<prompt>"\` — works with
                                 the `claude` CLI and `codex exec` style tools.
  --no-checkout                  Don't sync main / create the branch — just
                                 print the prompt and run the agent against
                                 the current branch.

Env:
  WORK_REPO=owner/name           Override target repo (default: upstream remote,
                                 falls back to origin). Same resolution as
                                 scripts/review.
  WORK_BRANCH_PREFIX=issue       Branch name is <prefix>/<num>-<slug> (default:
                                 issue).
EOF
}

cmd="${1:-}"
if [ -z "$cmd" ] || [ "$cmd" = "-h" ] || [ "$cmd" = "--help" ]; then
  usage
  exit 0
fi

# `pnpm work 1234 …` — first arg is numeric → implicit `start`.
case "$cmd" in
  ''|*[!0-9]*)
    case "$cmd" in
      start)
        shift
        exec "$here/start.sh" "$@"
        ;;
      *)
        echo "[work] unknown command: $cmd" >&2
        usage >&2
        exit 1
        ;;
    esac
    ;;
  *)
    exec "$here/start.sh" "$@"
    ;;
esac
