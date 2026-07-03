# TinyAgents Harness Migration Audit

Date: 2026-07-01

Scope: current `openhuman-5` checkout on branch `issue/4249-finish-tinyagents-migration`.

TinyAgents refresh: reviewed `tinyhumansai/tinyagents` `main` at
`348a0e7dc71a1f9039f3d523a2a384661a7a9acd` after the current audit was first
written. That repo is now materially ahead of the assumptions in the older
OpenHuman migration docs: it has harness cache/store/session primitives,
sub-agent reuse, graph subgraph/sub-agent nodes, lineage-aware events/status,
and JSONL-backed append stores.

This is a documentation-only audit of how much of the OpenHuman agent harness is
actually migrated to TinyAgents, and which remaining OpenHuman files are good
candidates to port, collapse, or keep as product-specific adapters.

## Bottom Line

The core turn loop is migrated. The live chat turn, channel/CLI turn, and
sub-agent turn all route through `src/openhuman/tinyagents::run_turn_via_tinyagents_shared`.
That means the model/tool iteration loop, TinyAgents middleware stack, event
bridge, stop hooks, context compression, unknown-tool recovery, and tool policy
boundary are on the TinyAgents harness path.

The repository still has a large OpenHuman harness shell around that path:

| Area                                 | Rust files |  Lines | Current role                                                                                                                                          |
| ------------------------------------ | ---------: | -----: | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/openhuman/agent/harness/`       |         83 | 30,521 | Agent session assembly, transcript compatibility, prompt/tool filtering, context product logic, sub-agent build pipeline, leftover generic loop seams |
| `src/openhuman/tinyagents/`          |         13 |  6,481 | TinyAgents adapters, middleware, event bridge, graph helpers, checkpoint adapter                                                                      |
| `src/openhuman/agent_orchestration/` |         58 | 21,704 | Product orchestration tools, durable workflow/team/session state, graph-backed fanout/delegation wrappers                                             |

The migration should not be read as "OpenHuman can delete `agent/harness/` now."
The correct read is: the execution core is on TinyAgents, but much of the
surrounding runtime was preserved for product compatibility. After reviewing the
current TinyAgents code, more of that surrounding runtime is now portable than
this audit originally assumed: session transcript tracking, prompt/response
cache handling, and sub-agent pipeline orchestration can be expressed through
TinyAgents store/cache/subgraph/sub-agent-node primitives, with OpenHuman keeping
only product policy and compatibility adapters.

## What Is Migrated

### Turn execution chokepoints

- `src/openhuman/agent/harness/session/turn/core.rs` routes the main chat turn
  through `super::graph::run_chat_turn_graph`, which is a thin wrapper over
  `run_turn_via_tinyagents_shared`.
- `src/openhuman/agent/harness/graph.rs` routes channel/CLI turns through
  `run_turn_via_tinyagents_shared`.
- `src/openhuman/agent/harness/subagent_runner/ops/graph.rs` routes sub-agent
  turns through `run_turn_via_tinyagents_shared`.
- `src/openhuman/tinyagents/mod.rs` owns the shared harness assembly:
  OpenHuman provider adapter, tool adapters, event bridge, middleware, steering,
  early-exit hooks, stop hooks, model/tool call caps, and outcome capture.

Practical status: the old in-tree model/tool loop is no longer the live
execution engine for those three entry points.

### Middleware and policy

Already migrated into TinyAgents middleware or the TinyAgents adapter seam:

- Approval and security gating at the tool boundary.
- CLI/RPC-only tool denial.
- Channel permission ceiling and tool policy checks.
- Unknown-tool recovery via an internal sentinel.
- Non-object tool-argument recovery.
- Cost budget pre-checks before model calls.
- Repeated tool failure halting.
- Context compression and message trimming via TinyAgents middleware.
- OpenHuman tool-output budgets and payload summarizer hooks as TinyAgents
  tool middleware.
- Stop-hook checks via `src/openhuman/tinyagents/stop_hooks.rs`.

This is real migration, but not complete ownership transfer: OpenHuman still
keeps rich policy metadata outside TinyAgents because the SDK tool schema does
not yet carry all OpenHuman policy fields.

### Graph-backed orchestration

The codebase has several real TinyAgents graph uses:

- `src/openhuman/tinyagents/delegation.rs`: durable plan -> execute -> review ->
  finalize graph.
- `src/openhuman/agent_orchestration/workflow_runs/graph.rs`: workflow phase DAG
  scheduler graph.
- `src/openhuman/agent_orchestration/agent_teams/graph.rs`: member execution
  conditional-routing graph.
- `src/openhuman/tinyagents/orchestration.rs`: reusable `run_parallel_fanout`
  helper, used by `spawn_parallel_agents` and workflow phase fanout.
- `src/openhuman/tinyagents/topology.rs`: topology export for fixed graph
  structures.
- `src/openhuman/agent_orchestration/running_subagents.rs`: detached sub-agent
  lifecycle is mirrored into TinyAgents `InMemoryTaskStore`.

Practical status: graph adoption is meaningful but uneven. The larger product
tools still own validation, persistence, cancellation semantics, compatibility
events, and result formatting.

## What Is Still Mostly OpenHuman-Owned

### Session, transcript, and cache shell

Largest remaining harness surface: `src/openhuman/agent/harness/session/`
at roughly 13.2k lines.

Current OpenHuman role:

- Session transcript migration and compatibility.
- Product-level `AgentBuilder` configuration.
- Prompt section assembly and prompt-cache stability decisions.
- Memory/context injection policy.
- Post-turn hooks, transcript persistence, and OpenHuman-specific history shape.
- Tool dispatcher compatibility for persisted native/XML/P-format transcript
  suffixes.

Updated TinyAgents target:

- Use TinyAgents `harness::store::{Store, AppendStore, FileStore,
  JsonlAppendStore}` as the durable substrate for message history, event
  journals, tool/model call records, artifacts, and local migration outputs.
- Use TinyAgents `harness::cache::{ResponseCache, PromptCacheLayout,
  CacheLayoutEvent, CachePolicy}` for response-cache and provider KV-cache
  layout protection instead of keeping prompt-cache reasoning as OpenHuman-only
  session logic.
- Use TinyAgents run/event/status lineage (`root_run_id`, `parent_run_id`,
  offsets, status stores) as the canonical internal transcript/run inspection
  surface, then project into legacy OpenHuman views where needed.
- Write a one-time migration script for old OpenHuman `session_raw/*.jsonl` and
  legacy Markdown session files into TinyAgents store/journal records. The
  script should preserve original session ids, transcript stems, timestamps,
  provider/model metadata, tool-call ids, and parent/child links, and should be
  idempotent with a marker/version row so it can be safely re-run.

Keep in OpenHuman:

- The legacy reader/writer compatibility layer until migrated sessions are
  proven equivalent.
- Product-specific prompt assembly, memory source identity, approval/security
  context, and JSON-RPC response shapes.

### Sub-agent build pipeline

Remaining surface: `src/openhuman/agent/harness/subagent_runner/`
at roughly 6.1k lines.

Keep in OpenHuman:

- Agent definition lookup and allowlist enforcement.
- Prompt assembly for archetypes.
- Parent context, memory context, action root, sandbox, and toolkit filtering.
- Worker-thread mirroring and transcript compatibility.
- Integrations-agent preflight and handoff cache behavior.

Updated TinyAgents target:

- Map the OpenHuman sub-agent build pipeline into TinyAgents `SubAgent`,
  `SubAgentSession`, and `SubAgentTool` once registry/policy adapters are ready.
- Express multi-step sub-agent work as graph structure using
  `graph::subagent_node` for harness-agent leaves and `graph::subgraph` for
  reusable pipelines. The current OpenHuman "prepare prompt -> filter tools ->
  run child -> checkpoint/handback -> mirror transcript" flow can become a
  subgraph rather than a sidecar runner.
- Use TinyAgents parent/root run lineage and `UsageTotals` for child usage/cost
  rollup instead of carrying separate OpenHuman aggregation structs.
- Keep OpenHuman-specific preflight nodes for agent definition lookup,
  allowlists, toolkit gates, sandbox/action-root narrowing, and worker-thread
  mirroring.

### Detached sub-agent registry

Remaining surface: `src/openhuman/agent_orchestration/running_subagents.rs`
at roughly 1.0k lines.

Current state: it uses TinyAgents `InMemoryTaskStore` as a typed lifecycle
ledger, but still owns watch channels, abort handles, wait/steer/cancel control,
tombstones, ownership checks, and session lookup.

Updated TinyAgents target:

- Durable `TaskStore` support.
- Typed wait/steer/cancel APIs.
- Parent/root run tree queries.
- Cancellation requests and hard abort lifecycle events.

TinyAgents now has more of the lifecycle substrate than this audit originally
assumed, including harness/graph status records, lineage-aware events,
`SubAgentSession` reuse, and append stores. The migration should therefore be an
adapter exercise first: map existing durable OpenHuman sub-agent session rows and
worker-thread records into TinyAgents session/status/journal records, then keep
OpenHuman controllers as compatibility projections.

Keep in OpenHuman until parity is proven:

- Desktop restart/resume compatibility.
- Existing JSON-RPC/tool response shapes.
- Durable OpenHuman session and worker-thread ledgers.

### Parallel agents and workflow fanout

Remaining surfaces:

- `src/openhuman/agent_orchestration/tools/spawn_parallel_agents.rs`
- `src/openhuman/agent_orchestration/workflow_runs/engine.rs`
- `src/openhuman/tinyagents/orchestration.rs`

Current state: fanout execution uses a TinyAgents graph helper, but
`spawn_parallel_agents` is not yet a first-class graph tool. It still owns
validation, worktree setup, overlap/stale-read checks, compatibility events, and
JSON output formatting.

Good TinyAgents candidates:

- Re-express `spawn_parallel_agents` as `validate -> dispatch -> worker ->
collect -> finalize` using graph `Send` or an SDK map/reduce helper.
- Move deterministic ordering, cancellation boundaries, graph status, and child
  lineage into TinyAgents graph state.
- Keep result formatting and OpenHuman worktree policy in a thin wrapper.

### Per-agent graph selectors

There are 32 `src/openhuman/agent_registry/agents/*/graph.rs` files and all
currently return `AgentGraph::Default`.

This is mostly scaffolding, not migrated behavior. It proves the extension seam
exists, but there are no bespoke per-agent TinyAgents graphs yet.

Good candidates for first real custom graphs:

- `orchestrator`: planning/delegation/parallelism policy can become explicit
  graph routing instead of prompt-only convention.
- `researcher`: search -> read -> synthesize -> cite can be bounded and
  checkpointed.
- `tool_maker`: detect missing capability -> generate -> validate -> expose can
  become a graph with explicit review gates.

Candidate cleanup: replace boilerplate default `graph.rs` files with registry
defaults once at least one real custom graph proves the API shape.

## Porting Candidates By Priority

### P0: Update stale active docs and comments

Evidence: active architecture docs still described the old `engine::run_turn_engine`
loop in sections below the TinyAgents status callout.

Action:

- Rewrite active sections to describe TinyAgents as the live loop.
- Move the removed in-house loop details into a short historical appendix.
- Sweep code comments that still say `run_turn_engine`, `run_tool_call_loop`, or
  `run_inner_loop` when they now mean the TinyAgents harness path.

Why first: stale docs make it hard to tell whether remaining files are live
runtime, compatibility shell, or historical residue.

Status (2026-07-01): done. `gitbooks/developing/architecture/agent-harness.md`
active sections now describe the TinyAgents path (loop, dialects-as-transcript-
compat, middleware context management, steering-channel cancellation, corrected
file map, 1.2 pin); pre-migration loop details are confined to the marked
historical sections. The retired-loop comment sweep landed earlier (f5a6b5196);
remaining `run_turn_engine`/`run_inner_loop` mentions in code are explicit
"legacy parity" references, not current-behavior claims.

### P1: Migrate old OpenHuman sessions into TinyAgents stores

Design: `docs/tinyagents-session-migration-design.md` (2026-07-01) — source
format inventory, target store layout, lineage-key mapping, idempotency
ledger, fixture matrix, and phasing.

Status (2026-07-01): Phase 1 (write-only importer) implemented in
`src/openhuman/session_import/` as `openhuman.session_import_run`, with the
full fixture matrix as tests. Phases 2–4 (read-side shadow, cutover,
retirement of legacy readers) remain.

Current OpenHuman files:

- `src/openhuman/agent/harness/session/transcript.rs`
- `src/openhuman/agent/harness/session/migration.rs`
- `session_raw/*.jsonl` and legacy Markdown session directories under user
  workspaces.

Target shape:

- Add a one-time migration command/script that reads old OpenHuman session JSONL
  and Markdown transcripts, normalizes them into TinyAgents message/event/store
  records, and writes them through TinyAgents-compatible `Store`/`AppendStore`
  semantics.
- Preserve compatibility metadata so old UI surfaces and run-ledger lookups can
  still answer by OpenHuman session key while new internals read by TinyAgents
  `thread_id`, `run_id`, `root_run_id`, and stream offset.
- Make the migration idempotent and observable: dry-run mode, per-file summary,
  warning list, migrated-count counters, and a marker/version record.

Risk: transcript shape is user data. This needs fixture coverage over current
flat `session_raw/*.jsonl`, older date-folder JSONL, Markdown sessions,
sub-agent transcript stems, native tool-call envelopes, XML/P-format tool
history, and malformed partial files.

### P2: Make event/status journals canonical

Current OpenHuman files:

- `src/openhuman/tinyagents/observability.rs`
- `src/openhuman/agent/harness/engine/progress.rs`
- `src/openhuman/session_db/run_ledger/*`
- `src/core/event_bus/*`

Target shape:

- TinyAgents journals/status stores become the internal source for model/tool
  events, usage, graph steps, child run lineage, and resumable inspection.
- `AgentProgress` and `DomainEvent` become compatibility projections.

Risk: this crosses UI streaming, cost footer, run ledgers, and desktop
reconnect behavior. It needs focused parity tests before deletion.

### P3: Port detached task lifecycle beyond the OpenHuman registry

Current OpenHuman files:

- `src/openhuman/agent_orchestration/running_subagents.rs`
- `src/openhuman/agent_orchestration/tools/{wait_subagent,steer_subagent,close_subagent,continue_subagent}.rs`

Target shape:

- TinyAgents owns typed task lifecycle, wait/steer/cancel semantics, parent/root
  run lineage, and terminal history.
- OpenHuman owns durable SQL/JSON projection and product response formatting.

This is no longer just waiting on SDK primitives. Current TinyAgents exposes
session reuse, lineage-aware events/status, graph/harness observability, and
JSONL append storage. The concrete blocker is now designing the OpenHuman
compatibility adapter and proving restart/resume parity against existing
controllers.

### P4: Re-express `spawn_parallel_agents` as a graph tool

Current OpenHuman files:

- `src/openhuman/agent_orchestration/tools/spawn_parallel_agents.rs`
- `src/openhuman/agent_orchestration/worktree.rs`
- `src/openhuman/tinyagents/orchestration.rs`

Target shape:

- A graph with nodes for validation, dispatch, worker, collect, finalize.
- Deterministic reducer state for per-task result order.
- Child run lineage and cancellation through graph status.
- Thin OpenHuman wrapper for the existing JSON result and worktree policy.

Why: this is a high-value migration because it converts a large product-visible
orchestration loop without touching the basic chat turn.

### P5: Replace generic checkpoint/progress seams

Current OpenHuman files:

- `src/openhuman/agent/harness/engine/checkpoint.rs`
- `src/openhuman/agent/harness/engine/progress.rs`
- `src/openhuman/agent/harness/subagent_runner/ops/checkpoint.rs`

Target shape:

- Model-call cap, early-exit pause, resumable checkpoint summary, and progress
  projection represented as TinyAgents status/events/middleware.

Keep only the OpenHuman-specific compatibility formatting for existing
transcripts and tool outputs.

### P6: Retire boilerplate per-agent graph selectors

Current OpenHuman files:

- `src/openhuman/agent/harness/agent_graph.rs`
- `src/openhuman/agent_registry/agents/*/graph.rs`

Target shape:

- Default graph supplied centrally.
- Only agents with custom graph behavior keep a `graph.rs`.
- Registry diagnostics show which graph each agent actually uses.

This should happen after at least one bespoke graph lands, so the cleanup does
not remove a seam before it has proved useful.

## Code That Should Probably Stay In OpenHuman

Do not port these blindly into TinyAgents:

- Agent registry definitions and built-in prompt semantics.
- Tool allowlists/denylists that encode product policy.
- Security policy, sandbox roots, approval records, credential access, and
  workspace/action-dir boundaries.
- Session transcript migration and persisted compatibility formats.
- OpenHuman model provider routing, billing classification, and credential
  ownership.
- UI-facing JSON-RPC response shapes and `DomainEvent` compatibility until every
  subscriber is moved.
- Worktree isolation policy and dirty-worktree safeguards.
- Memory stores, retrieval policy, and context source identity.

TinyAgents should own generic runtime machinery. OpenHuman should own product
semantics and compatibility boundaries.

## Main SDK Gaps Blocking More Deletion

The most important upstream gaps are tracked in `docs/tinyagents-sdk-gaps.md`,
but that file must be refreshed against TinyAgents `main` at
`348a0e7dc71a1f9039f3d523a2a384661a7a9acd`. Several older "missing" items now
exist as concrete TinyAgents modules:

- `harness::cache` has response caching and prompt/KV-cache layout protection.
- `harness::store` has key-value stores and `JsonlAppendStore`.
- `harness::subagent` has `SubAgent`, `SubAgentSession`, and `SubAgentTool`.
- `graph::subgraph` and `graph::subagent_node` can express nested graph and
  sub-agent pipelines.
- harness/graph observability and status types carry `root_run_id` /
  `parent_run_id` lineage.

Remaining blockers are narrower:

- Rich tool policy metadata in TinyAgents schemas.
- Recoverable unknown-tool policy without the OpenHuman sentinel.
- Full OpenHuman policy mapping for approval/security/sandbox/workspace roots.
- A migration adapter from OpenHuman session JSONL/Markdown/run-ledger rows into
  TinyAgents store/journal/status records.
- SQLite/native-link compatibility if OpenHuman wants to use TinyAgents'
  embedded SQLite backend directly rather than its own SQLite adapter.
- Redaction/cursor/backfill rules for production UI replay.
- OpenHuman controller compatibility during the migration window.

## Suggested Next Documentation Pass

1. Rewrite the active `gitbooks/developing/architecture/agent-harness.md`
   sections below the status block so they describe the TinyAgents path instead
   of the removed in-house loop.
2. Add a one-time transcript/session migration design for old OpenHuman
   `session_raw/*.jsonl` and Markdown sessions into TinyAgents store/journal
   records. Done: `docs/tinyagents-session-migration-design.md`.
3. Add a small "Historical pre-TinyAgents loop" appendix for details that are
   still useful context.
4. Add per-folder READMEs or module docs for:
   - `src/openhuman/agent/harness/session/`: product session shell over
     TinyAgents.
   - `src/openhuman/agent/harness/subagent_runner/`: OpenHuman sub-agent build
     pipeline over TinyAgents.
   - `src/openhuman/agent_orchestration/`: product orchestration wrappers over
     TinyAgents graphs/task lifecycle.
5. Keep `docs/tinyagents-migration-spec.md` as the backlog, and use this audit
   as the current inventory snapshot.
