# Subconscious Orchestration Layer — Multistage Plan

Implements the split-brain architecture in [`docs/arch-subconscious.md`](../../arch-subconscious.md)
on top of the **tinyagents graph harness** (`src/openhuman/tinyagents/` + `agent_graph`), with
**tiny.place DM channels** as the transport for external Claude Code / Codex sessions, surfaced in
the new **`TinyPlaceOrchestrationTab`** on the Brain page.

## How to use this folder (goal commands)

Each `stage-NN-*.md` file is a self-contained **goal**: hand a single stage file to an agent
(`claude "implement docs/plans/subconscious-orchestration/stage-03-core-ingest.md"`) and it has the
goal statement, the files to read first, deliverables, tasks, and acceptance criteria. Stages are
ordered by dependency; each stage must land (tests green, committed) before the next starts.
Stage 1 lands in the **`tiny.place/` checkout** (separate repo, separate PR); stage 2 spans both
repos (its wrapper half rides with stage 1); stages 3–8 land here.

## Design decision: one graph, two triggers

The whole **wake path is a single tinyagents graph** (mirroring the spec's one `StateGraph`), not
separate agents ping-ponging through the event bus:

```
invoke(session thread) ─► normalize ─► frontend(pass1) ─► reasoning/execute ─► compress
                                            ▲                                      │
                                            └── reply ◄── world_diff ◄─────────────┘
                          frontend(pass2) ─► send_dm ─► context_guard ─► END
```

- **One state object** (`OrchestrationState`: `messages`, `agent_instructions`, `agent_reply`,
  `channel_response`, `subconscious_steering`, `compressed_history`, `world_state_diff`,
  `context_utilization`) flows through conditional edges — the spec's routing predicate
  (`channel_response` present → wrap up, else execute) is a graph edge, not bus choreography.
- **One checkpointer** (`SqlRunLedgerCheckpointer`, thread id `orchestration:<session_id>`) gives
  crash-resume for the whole cycle and gives the subconscious a consistent snapshot to read.
- Only two things live **outside** the graph:
  1. **Transport/ingest** (stage 3): DMs arrive asynchronously; the subscriber persists + then
     *invokes* the graph on the session's thread. A polling node inside the graph would fight
     checkpointing.
  2. **Subconscious** (stage 6): out-of-band cron by design (the spec runs it via
     `update_state(as_node="subconscious_cron")`). It reads checkpointed state + the store and
     writes `subconscious_steering` back into the thread — a decoupled writer, not an edge.

Consequence for the stages: stage 4 builds the **graph skeleton + state + front-end nodes**,
stage 5 adds the **reasoning/memory nodes to the same graph**. Different model tiers per node are
fine — tinyagents nodes each resolve their own provider/model (`hint:chat` vs `hint:reasoning`).

## Target architecture → repo primitives

| Spec concept (`arch-subconscious.md`) | Repo primitive |
| --- | --- |
| Channels (ingestion boundary) | tiny.place Signal DMs → `src/openhuman/tinyplace/` (`messages_list`, `signal_send_message`, `streams.rs`) |
| DM authorization | tiny.place mutual **contact graph** (`backend-tinyplace-v2` `/contacts/*`; relay rejects non-contact DMs) → new `tinyplace_contacts_*` controllers + pairing flow (stage 2) |
| Front-End Agent (Quick LLM) | `frontend` nodes of the unified graph, model `hint:chat` (`src/openhuman/routing/policy.rs`) |
| Reasoning LLM (Orchestration Core) | `execute` node of the unified graph (`AgentGraph::Custom` runner, `hint:reasoning`), spawning sub-agents via `subagent_runner` |
| Bi-directional loop (frontend ⇄ reasoning) | Conditional edges on `agent_reply` / `channel_response` in the single graph state |
| Checkpointing / resume | `SqlRunLedgerCheckpointer` (`src/openhuman/tinyagents/checkpoint.rs`), keyed by orchestration `thread_id` |
| 20:1 compression hook | tinyagents middleware (pattern: `summarize.rs`, `TurnContextMiddleware`) writing `compressed_history` |
| World state diff | New `orchestration` store table, appended per execution cycle |
| 80–90% context hooks | Context-window middleware (extend `summarize.rs` seam) + eviction to memory/embeddings |
| Subconscious LLM + cron | Existing `SubconsciousEngine` tick (`src/openhuman/subconscious/engine.rs`, `heartbeat/`), gated by `scheduler_gate` |
| Steering directive injection | New steering store read by the Reasoning agent's `prompt.rs::build` |
| UI surface | `TinyPlaceOrchestrationTab` (Brain → Orchestration group), backed by real metadata instead of string heuristics |

## Message flow (end to end)

```
Claude Code / Codex instance
  └─ tinyplace wrapper (stage 1) — tails session JSONL → SessionEnvelope
       └─ contact pairing (stage 2) — accepted mutual contact edge; relay refuses DMs
          between non-contacts (403 not_a_contact), so this gates everything below
       └─ Signal E2E DM → owner agent's tiny.place inbox           [tagged: sessionId, source, role, kind]
            └─ core ingest (stage 3) — orchestration::ingest normalizes DMs → SessionState
                 └─ Front-End nodes (stage 4, Quick LLM) — macro-instructions
                      └─ Reasoning + memory nodes (stage 5) — sub-agents, 20:1 compression,
                         world-state diff, context hooks; replies upstream
                 └─ Front-End pass 2 — channel_response → Signal DM back to session
            └─ Subconscious tick (stage 6, cron/heartbeat) — reads compressed history + world diff,
               writes steering directives → injected into Reasoning prompts next run
  UI: TinyPlaceOrchestrationTab (stage 7) — pairing, master / subconscious / per-session chat windows
```

## Stage index

| Stage | Repo | Delivers | Depends on |
| --- | --- | --- | --- |
| [1 — tiny.place session bridge](stage-01-tinyplace-session-bridge.md) | `tiny.place/` | `SessionEnvelope` v1 schema; Claude Code wrapper; DM forwarding; wrapper pairing handshake | — |
| [2 — contact pairing & DM authorization](stage-02-contact-pairing.md) | both | Contacts RPC in core; user-consented link/approve flows; Brain-tab pairing UX | 1 |
| [3 — core ingest + session state](stage-03-core-ingest.md) | openhuman | `orchestration` domain: DM ingest, session store, typed envelopes | 1 (2 for live traffic) |
| [4 — graph skeleton + front-end nodes](stage-04-frontend-agent.md) | openhuman | Unified `OrchestrationState` graph, checkpointing, two-pass frontend nodes (Quick LLM) | 3 |
| [5 — reasoning + memory nodes](stage-05-reasoning-orchestrator.md) | openhuman | `execute`/`compress`/`world_diff`/`context_guard` nodes in the same graph, sub-agent spawning | 4 |
| [6 — subconscious steering loop](stage-06-subconscious-steering.md) | openhuman | Steering directives from world diff via existing heartbeat/cron | 5 |
| [7 — RPC + UI wiring](stage-07-rpc-and-ui.md) | openhuman | `orchestration.*` RPC; `TinyPlaceOrchestrationTab` on real data + streaming + composer | 3, 6 |
| [8 — testing & observability](stage-08-testing-observability.md) | both | json_rpc_e2e, Vitest, debug logging audit, coverage gate | all |

## Global invariants (apply to every stage)

- **Loop continuity**: routing must terminate on `channel_response` presence — never allow
  frontend ⇄ reasoning infinite cycling (spec §5 checklist).
- **20:1 strictness**: compression output budget = ⌈input_tokens / 20⌉, enforced, not advisory.
- **World diff is append-only**: sequential timeline from genesis; never wipe keys per cycle.
- **Context hooks run before END**: eviction takes effect prior to the next iteration.
- **Subconscious isolation**: it never talks to channels or users directly; its only output is
  steering directives (and it keeps `SubconsciousTainted` origin so effect tools stay gated).
- **Pairing consent**: DMs only flow over user-consented contact edges — the wrapper auto-accepts
  only its configured owner identity; OpenHuman never auto-accepts an unsolicited request unless
  the user initiated the link or explicitly opted in via config.
- **Security**: tinyplace identity stays wallet-derived (`wallet::tinyplace_signer_seed()`); never
  log message bodies or seeds; Signal bodies stay E2E — the relay sees ciphertext only.
- **Repo rules**: new Rust code in a dedicated domain dir (canonical module shape), controller
  registry (no `dispatch.rs` branches), verbose `[orchestration]`-prefixed debug logging, i18n for
  all UI strings across all 14 locales, ≥80% changed-line coverage.
