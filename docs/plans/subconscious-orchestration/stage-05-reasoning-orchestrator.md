# Stage 5 — Reasoning + memory nodes (completing the unified graph)

## Goal command

> Replace the stage-4 stubs with the real **reasoning and memory nodes** of the unified
> orchestration graph: an `execute` node on the reasoning tier that applies the current
> subconscious steering directive and spawns execution sub-agents via `subagent_runner`, followed
> by the three memory mechanics from the spec as their own nodes — the **20:1 compression hook**
> over the cycle's execution trace, the **append-only world-state diff**, and the **80–90%
> context-eviction guard** that runs after mutations and before END. All state changes flow
> through `OrchestrationState` and are checkpointed; durable copies land in the orchestration
> store for the subconscious (stage 6) and the UI (stage 7).

## Read first

- Stage 4's `orchestration/graph/` (state, topology, stubs to replace).
- `src/openhuman/tinyagents/` — `mod.rs` (`run_turn_via_tinyagents_shared`, `RunPolicy`, steering
  forwarders), `delegation.rs` (sub-agent spawning), `summarize.rs` (tokenizer + summarization
  seam), `middleware.rs` (`TurnContextMiddleware`, `SuperContextConfig`), `observability.rs`
  (`OpenhumanEventBridge`, `CapPauser`, `GraphTracingSink`).
- `src/openhuman/agent/harness/subagent_runner/ops/graph.rs` — `run_subagent_via_graph`.
- `docs/arch-subconscious.md` §3–§5 — compression ratio, world diff shape, guardrail checklist.

## Deliverables

1. **`execute` node** driven by `orchestration/reasoning_agent/{agent.toml, prompt.md}`
   (`agent_tier = "reasoning"`, `hint:reasoning`, large context):
   - System prompt assembled per cycle: base prompt + `subconscious_steering` from state (default
     alignment string when empty).
   - Tools: sub-agent spawn (via the `delegation.rs` seam; steering text threaded into sub-agent
     definitions as steering traits, spec §3.2; concurrent cap config, default 2) + the tinyplace
     read tools already whitelisted for the orchestrator agent.
   - Output: sets `agent_reply` in state; raw trace (assistant text, tool calls/results, sub-agent
     outputs) captured for the compression node.
2. **`compress` node** (`orchestration/graph/compress.rs`): token-count the cycle trace (tokenizer
   util used by `summarize.rs`); summarize via a cheap `hint:*` route with an enforced output
   budget of `min(input_tokens / 20, input_tokens)`; apply the 200-token floor only when the
   source trace is large enough that the budget is still compressive. Retry once if >1.5× budget,
   then hard-truncate. Append to `state.compressed_history` **and** persist to the store's
   `compressed_history` table (cycle id, session id, token counts, text).
3. **`world_diff` node** (`orchestration/graph/world_diff.rs` + store table): append one timeline
   entry `{ seq, cycle_id, event_signature, world_mutation, delta, timestamp }`; `genesis` row on
   first cycle; `terminal_state` kv updated per cycle. Append-only — never rewritten.
4. **`context_guard` node**: utilization from accumulated `AgentTurnUsage` vs the resolved context
   window, stored in `state.context_utilization`; at ≥ threshold
   (`[orchestration] context_evict_threshold = 0.85`, clamped 0.8–0.9) map-reduce-summarize the
   oldest compressed-history entries, push summaries to the memory domain
   (`metadata.path_scope = "orchestration/<session_id>"` for RAG), drop them from state, reset
   utilization. Runs after all mutations, before END (spec invariant — edge ordering test).
5. **Observability**: node progress mirrored through `OpenhumanEventBridge` (cycles + sub-agents
   visible in the agent-observability UI with usage/cost); `scheduler_gate::wait_for_capacity()`
   before every model call (execute, compress, evict).

## Tasks

1. Reasoning agent package + swap `execute_stub` → `execute`; mock-provider test: instructions in
   → steering applied (assert in captured system prompt) → `agent_reply` set.
2. Sub-agent spawning path with a stub sub-agent; steering-trait threading test.
3. `compress` node with ratio-enforcement tests (large fixture trace → ≤ budget×1.5; floor case;
   store row written).
4. `world_diff` node + append-only property test (two cycles → seq 1,2; genesis untouched).
5. `context_guard` tests at 0.84 (no-op) and 0.86 (evicts: state shrinks, memory write happened,
   utilization reset) + guard-before-END edge-ordering test.
6. Full-graph e2e (mock providers): DM → normalize → frontend(1) → execute (sub-agent stub) →
   compress → world_diff → frontend(2) → send_dm → guard → END; exactly one compressed row + one
   diff entry per cycle; checkpoint resume mid-cycle produces no duplicate side effects.

## Acceptance criteria

- Full autonomous cycle passes hermetically with exactly one outbound DM, one compressed-history
  row, one world-diff entry.
- All four spec §5 guardrails have a dedicated test (loop continuity, 20:1 strictness, append-only
  diff, guard-before-END ordering).
- Kill/restart mid-cycle resumes from the last checkpoint without duplicate outbound DMs or
  duplicate store rows.
- `pnpm test:rust` green; cycles visible in agent observability with cost/usage totals.
