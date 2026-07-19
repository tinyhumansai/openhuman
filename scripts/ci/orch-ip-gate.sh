#!/usr/bin/env bash
#
# CI gate: the local orchestration "brain" (reasoning/wake graph, its prompt
# assets, and per-agent model-selection metadata) moved server-side. This gate
# fails the build if any of that proprietary IP re-enters the open client repo.
#
# Run from anywhere; resolves the repo root itself.
set -euo pipefail
cd "$(git rev-parse --show-toplevel 2>/dev/null || echo .)"

fail=0
note() {
  echo "orch-ip-gate FAIL: $1"
  fail=1
}

# 1. The local wake/reasoning graph and its per-agent packages.
[ -d src/openhuman/orchestration/graph ] && note "orchestration/graph/ (local wake graph) present"
for d in reasoning_agent frontend_agent master_agent master_reporter; do
  [ -d "src/openhuman/orchestration/$d" ] && note "orchestration/$d agent package present"
done

# 2. The retired tiny.place subconscious steering profile.
[ -f src/openhuman/subconscious/profiles/tinyplace.rs ] &&
  note "tinyplace subconscious profile present"

# 3. Any orchestration prompt.md asset (the proprietary prompt IP).
if find src/openhuman/orchestration -name 'prompt.md' -print -quit 2>/dev/null | grep -q .; then
  note "orchestration prompt.md asset present"
fi

# 4. Local wake-graph / reasoning-runtime symbols.
if grep -rqE 'ProductionRuntime|run_orchestration_graph|OrchestrationRuntime|orchestration_graph_topology' \
  src/openhuman/orchestration 2>/dev/null; then
  note "local wake-graph symbol present in orchestration/"
fi

# 5. Per-agent model-selection / tier-routing metadata.
if grep -rqE 'agent_tier|^\s*\[model\]' src/openhuman/orchestration --include='*.toml' 2>/dev/null; then
  note "model-selection metadata (agent_tier / [model]) in orchestration/*.toml"
fi

if [ "$fail" -ne 0 ]; then
  echo ""
  echo "The retired local orchestration brain (reasoning graph / prompts / model"
  echo "routing) must not re-enter the open repo — it runs in tinyhumansai/backend."
  exit 1
fi

echo "orch-ip-gate: clean — no local orchestration brain IP present"
