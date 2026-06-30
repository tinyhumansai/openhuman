#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$ROOT_DIR"

if [[ -f "$ROOT_DIR/.env" ]]; then
  # shellcheck source=../load-dotenv.sh
  source "$ROOT_DIR/scripts/load-dotenv.sh" "$ROOT_DIR/.env"
fi

export RUST_LOG="${RUST_LOG:-info,spawn_subagent=debug,openhuman_core::openhuman::agent=debug,openhuman_core::openhuman::agent_orchestration=debug}"

echo "[harness_subagent_audit] running live audit; requires configured provider/backend credentials" >&2
exec cargo run --manifest-path "$ROOT_DIR/Cargo.toml" --bin harness-subagent-audit -- "$@"
