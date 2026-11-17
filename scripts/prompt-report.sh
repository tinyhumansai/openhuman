#!/usr/bin/env bash
# Print every agent's fixed per-turn prefix — system prompt plus advertised tool
# schemas — one row per agent, largest first, then their sum (a size measure,
# not a cost: each prefix is paid only on the turns that agent runs).
#
# Report-only: nothing here fails on a number. It consumes the JSON from
# `scripts/prompt-size-measure.sh`.
#
# Usage: scripts/prompt-report.sh [--workspace <dir>]
#
#   (default)          Hermetic: a fresh empty workspace and config, so the
#                      numbers describe the repo, not whoever is logged in.
#   --workspace <dir>  Measure a real, signed-in workspace instead.
#
# Units are bytes. `~tok` is bytes / `EST_BYTES_PER_TOKEN` (agent/cli/debug/
# prompt_size.rs, read at run time), the same reading aid `prompt-size` prints —
# a divisor, not a tokenizer. A real token count is true for one model only, and the fleet
# spans several.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

REAL_WORKSPACE=""
while (( $# )); do
  case "$1" in
    --workspace) REAL_WORKSPACE="${2:?--workspace needs a directory}"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 64 ;;
  esac
done

# The divisor is read from the Rust constant so the two cannot drift.
TOK="$(grep -oE 'EST_BYTES_PER_TOKEN: usize = [0-9]+' \
  crates/openhuman-core/src/agent/cli/debug/prompt_size.rs | grep -oE '[0-9]+$')" \
  || { echo "EST_BYTES_PER_TOKEN not found in prompt_size.rs" >&2; exit 1; }

if [[ -n "$REAL_WORKSPACE" ]]; then
  echo "[prompt-report] measuring signed-in workspace $REAL_WORKSPACE" >&2
  measured="$(bash scripts/prompt-size-measure.sh --workspace "$REAL_WORKSPACE")"
else
  measured="$(bash scripts/prompt-size-measure.sh)"
fi

TOK="$TOK" python3 - "$measured" <<'PY'
import json, os, sys

TOK = int(os.environ["TOK"])  # EST_BYTES_PER_TOKEN
rows = []
for r in json.loads(sys.argv[1])["agents"]:
    name = r["agent"]
    worst = max(r["tools"], key=lambda t: t["bytes"], default=None)
    rows.append((name, r["prompt_bytes"], r["tool_bytes"], r["fixed_prefix_bytes"],
                 f"{worst['name']} ({worst['bytes']})" if worst else "-"))
rows.sort(key=lambda row: -row[3])

fmt = "{:<34} {:>9} {:>9} {:>9} {:>8}  {}"
print(fmt.format("agent", "prompt B", "tools B", "fixed B", "~tok", "worst tool (B)"))
for name, p, t, f, w in rows:
    print(fmt.format(name, p, t, f, f // TOK, w))
tp, tt, tf = (sum(row[i] for row in rows) for i in (1, 2, 3))
# A sum no single turn pays: each prefix is paid only when that agent runs.
print(fmt.format(f"sum of {len(rows)} (not a turn cost)", tp, tt, tf, tf // TOK, ""))
PY
