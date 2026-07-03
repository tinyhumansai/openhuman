# Subconscious orchestration layer

The orchestration layer is OpenHuman's **split-brain** coordinator for wrapped
Claude Code / Codex sessions that talk to the owner agent over tiny.place
Signal-encrypted DMs. It turns each inbound session DM into one autonomous
**wake cycle** driven by a single `tinyagents` graph, keeps a durable per-session
chat model, and runs an offline **subconscious** that reflects on how the world
is trending and steers later cycles.

Domain root: [`src/openhuman/orchestration/`](../../../src/openhuman/orchestration).
Design spec: [`docs/arch-subconscious.md`](../../../docs/arch-subconscious.md) and
the staged plan under [`docs/plans/subconscious-orchestration/`](../../../docs/plans/subconscious-orchestration).

## End-to-end flow

```
Claude Code / Codex session
  └─ tinyplace harness wrapper — tails the session JSONL → SessionEnvelopeV1
       └─ Signal E2E DM → owner agent's tiny.place inbox   [tagged sessionId, source, role]
            └─ ingest (decrypt-once → classify → persist → ack)     orchestration/ingest.rs
                 └─ OrchestrationSessionMessage event → debounced wake
                      └─ THE WAKE GRAPH (one tinyagents CompiledGraph)  orchestration/graph/
                           normalize → frontend(1) → execute → compress → world_diff
                                          ▲                                   │
                                          └───────────────────────────────────┘
                                          │
                                          └─(channel_response)─► send_dm ─► context_guard ─► done
            └─ subconscious tick (offline, cron/heartbeat) — reviews compressed history +
               cumulative world diff → emits a steering directive that later cycles inject
  UI: Brain → Orchestration tab (orchestration.* RPC + orchestration:message socket)
```

## The wake graph (stages 4–5)

One `OrchestrationState` (`graph/state.rs`) flows through the whole cycle and is
checkpointed at every super-step boundary under thread `orchestration:<session_id>`
by `SqlRunLedgerCheckpointer`. `frontend` is the router (command-routing): when
`channel_response` is present it wraps up (`send_dm`), otherwise it hands macro
instructions to `execute` and loops back. The reasoning core always sets
`agent_reply`, so the second front-end pass compiles a `channel_response`; a hard
`max_supersteps` backstop guarantees termination (spec §5 loop continuity).

Every behaviour-bearing node — the two-pass front end (Quick LLM, `hint:chat`),
the reasoning core (`hint:reasoning`, spawns worker sub-agents), 20:1 compression,
the append-only world-state diff, utilization + eviction, and the DM reply — is
bundled behind one injected `OrchestrationRuntime` (`graph/mod.rs`), so the graph
mechanics are hermetically unit-testable with a single stub while production wires
the real agents / store / memory in `ops.rs`.

Memory mechanics (spec §3–§4):

- **`compress`** condenses the cycle's execution trace to a strict `input/20` token
  budget (200-token floor only when still compressive), retry-once-then-truncate,
  persisted idempotently by `cycle_id` to the `compressed_history` table.
- **`world_diff`** appends one entry to an append-only timeline (monotonic `seq`
  from genesis, `terminal_state` kv), idempotent by `cycle_id`.
- **`context_guard`** runs after all mutations, before END: at ≥
  `context_evict_threshold` (clamped 0.8–0.9) it evicts the oldest compressed
  entries to memory RAG under `path_scope = orchestration/<session>` and resets
  utilization.

## The subconscious steering loop (stage 6)

The existing `SubconsciousEngine` tick gains an `orchestration_review` stage that
runs **fully offline** — a single tool-free provider chat on the `subconscious`
route under `SubconsciousTainted` origin. It reads unreviewed `compressed_history`
plus the cumulative world-diff timeline and emits at most one dense
`STEERING_DIRECTIVE` (with `expires_after_cycles`) into the append-only
`steering_directives` store (supersede chain + cycle-count expiry). At the start of
each wake cycle `ops::seed_state` bumps a global reasoning-cycle counter and loads
the current non-expired directive into `state.subconscious_steering`, which the
`execute` node weaves into its system prompt via a task-local. The subconscious is
a decoupled writer — never an edge in the wake graph, never a channel/effect.

## RPC + UI (stage 7)

Renderer-only controllers (internal registry) in `orchestration/schemas.rs`:
`openhuman.orchestration_{sessions_list, messages_list, send_master_message,
mark_read, status}`. Live updates ride an `orchestration:message` socket event
(`bus.rs` broadcast → `core/socketio.rs` bridge) fanned out for every persisted
chat message. The Brain → Orchestration tab (`TinyPlaceOrchestrationTab.tsx` +
`useOrchestrationChats.ts`) reads real store classification, live-updates, and lets
the owner steer the front-end agent from the Master composer.

## Running unattended (stage 8)

- **No message loss**: ingest dedupes by relay `message_id` *before* decrypt (the
  Signal ratchet is never advanced twice); a relay/decrypt error leaves the message
  un-acked for a clean retry.
- **No duplicate DM**: the idempotence cursor advances only after a completed,
  DM-sent cycle; `dm_sent` is checkpointed and the deterministic `cycle_id` keeps
  the `compressed_history` / `world_diff` store writes idempotent across a
  checkpoint resume.
- **Backpressure**: each cycle awaits `scheduler_gate::wait_for_capacity()`, so a
  `Paused`/`Throttled` gate defers the cycle rather than dropping it.
- **Malformed input**: any non-envelope / malformed DM body falls back to the peer's
  Master window; the parser never panics.
- **Observability**: nodes log entry/exit with `session_id` / `cycle_id` /
  `tick_id` correlation ids; `orchestration.status` exposes the current steering
  directive, last subconscious tick, ingest-cursor lag, and last error. Message
  bodies / decrypted plaintext / seeds are never logged (guarded by a source-scan
  test).

## Configuration

The `[orchestration]` config block (`src/openhuman/config/schema/orchestration.rs`):
`enabled`, `debounce_ms`, `max_supersteps`, `message_window`,
`context_evict_threshold` (clamped 0.8–0.9), `subagent_concurrency`.
