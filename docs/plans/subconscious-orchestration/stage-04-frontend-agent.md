# Stage 4 — Unified orchestration graph: skeleton, state & front-end nodes

## Goal command

> Build the **single orchestration graph** that carries the whole wake path, and implement its
> **front-end (Quick LLM) nodes**. Define the shared `OrchestrationState`, the graph topology with
> conditional routing, checkpointing on thread id `orchestration:<session_id>`, and the two-pass
> front-end behavior: pass 1 turns raw session/master traffic into `agent_instructions` and routes
> down; pass 2 (after the reasoning node sets `agent_reply`, stubbed in this stage) compiles
> `channel_response` and sends it back to the originating tiny.place DM. No human gate — the loop
> is autonomous, and the routing predicate must terminate (spec §5 loop continuity).

## Read first

- `docs/plans/subconscious-orchestration/README.md` — "one graph, two triggers" design decision.
- `src/openhuman/agent/harness/agent_graph.rs` — `AgentGraph::Custom`, `AgentTurnRequest`.
- `src/openhuman/tinyagents/` — `mod.rs` (shared runner), `checkpoint.rs`
  (`SqlRunLedgerCheckpointer`), `topology.rs`/`orchestration.rs` (existing graph-composition
  precedents — follow whichever expresses multi-node graphs; document the choice), `tools.rs`
  (`EarlyExit` seam), `model.rs` (per-node provider/model resolution).
- `src/openhuman/subconscious/agent/` — slim-agent packaging (agent.toml + prompt.md + graph.rs).
- `src/openhuman/routing/policy.rs` — `hint:chat` (quick, remote for TTFT) vs `hint:reasoning`.
- `src/openhuman/tinyplace/schemas.rs` — `handle_tinyplace_signal_send_message` (reply path).
- `docs/arch-subconscious.md` §2.2, §4 — node/edge reference topology.

## Deliverables

1. **Graph state** (`orchestration/graph/state.rs`): `OrchestrationState` — `messages` (windowed
   from the stage-3 store), `agent_instructions: Option<String>`, `agent_reply: Option<String>`,
   `channel_response: Option<String>`, `subconscious_steering: Option<String>` (read in stage 5/6),
   `compressed_history: Vec<CompressedEntry>`, `world_state_diff: WorldDiff`,
   `context_utilization: f32`. Serde-serializable so `SqlRunLedgerCheckpointer<OrchestrationState>`
   persists it at superstep boundaries.
2. **Graph topology** (`orchestration/graph/mod.rs`), registered as the orchestration agent's
   `AgentGraph::Custom` runner:
   - Nodes this stage: `normalize` (fold pending session messages into state),
     `frontend` (two-pass, Quick LLM), `send_dm` (Signal reply to the session counterpart),
     `execute_stub` (sets a canned `agent_reply`; replaced in stage 5), `context_guard`.
   - Conditional edges = the spec's router: from `frontend`, `channel_response` present →
     `send_dm` → `context_guard` → END; else → `execute` → back to `frontend`.
   - **Invocation**: `invoke_orchestration_graph(session_id)` in `orchestration/ops.rs`, called by
     the stage-3 ingest subscriber on `OrchestrationSessionMessage`, debounced per session so DM
     bursts produce one graph run. Resumes from the last checkpoint for that thread.
3. **Front-end node** driven by a slim agent package
   `orchestration/frontend_agent/{agent.toml, prompt.md}`: model `hint:chat`, small context
   budget, tool surface limited to `defer_to_orchestrator` (EarlyExit emitting
   `agent_instructions`) and `reply_to_channel` (emits `channel_response`); pass selection purely
   from state (`agent_reply` present → pass 2), mirroring `frontend_agent_node` in the spec.
4. **Gating**: graph runs with background origin (no interactive approval parking); each
   LLM-bearing node awaits `scheduler_gate::wait_for_capacity()`.
5. **Logging**: `[orchestration]` node entry/exit with `session_id`, node name, pass number,
   routing decision at debug.

## Tasks

1. State + checkpointer round-trip test (serialize → resume → identical state).
2. Topology with stubs; graph-level test: one invocation walks
   normalize → frontend(1) → execute_stub → frontend(2) → send_dm → guard → END, sending exactly
   one DM (mock tinyplace op).
3. Frontend agent package (prompt.md with two-pass contract + macro-instruction format,
   agent.toml) + loader registration; tools in `orchestration/tools.rs` (domain-owned rule).
4. Debounced invocation from the ingest subscriber; idempotence test (re-invoke with no new
   messages → no LLM call, no DM).
5. Loop-continuity property test: adversarial state combos (`agent_reply` + `channel_response`
   both set, neither set, instructions without reply) never cycle more than the configured max
   supersteps and never double-send.

## Acceptance criteria

- A new session DM triggers one full stubbed cycle ending in exactly one outbound Signal DM to the
  right counterpart; checkpoints exist for the thread.
- Quick tier verified: frontend node requests resolve via `hint:chat`; prompt+history within the
  small budget (asserted).
- Kill/restart mid-run resumes from checkpoint without a duplicate DM.
- `pnpm test:rust` green; no infinite cycling under repeated triggers.
