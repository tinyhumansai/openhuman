# 99 — Deletion ledger (master list)

Hard-migration means these files GO. Each row names its precondition step.
Rule: call-site search + parity coverage before every delete; tick rows as
they land.

## Immediately deletable (dead/vestigial — no precondition beyond call-site search)

- [x] `agent/memory_loader.rs` (5-line facade) — 09
- [x] `agent/tree_loader.rs` (210, unwired per #3170) — 09
- [x] `harness/compaction/mod.rs` shim — 03.1

## Superseded by existing middlewares (verify + delete)

- [x] `harness/compaction/cache_align.rs` (200) — 03.1/03.2
- [x] `tokenjuice::compact_tool_output` default-Full wrapper — 01.4
- [x] `context/microcompact.rs` (269) — 03.1
- [x] `context/pipeline.rs` (454) + `context/guard.rs` (236, keep stats structs) — 03.1
- [x] `context/tool_result_budget.rs` (172) — 03.1
- [x] `harness/payload_summarizer.rs` (490) — 01.4

## Deletable after SDK-surface adoption

- [x] `UNKNOWN_TOOL_SENTINEL` + `UnknownToolRewriteMiddleware` — 01.2
- [ ] crate-internal tool side-lookup in `tinyagents/middleware.rs` — 01.1
      (live overlays for args-aware effects/permissions, CLI/RPC scope, and
      generated runtime context until SDK policy metadata can represent them)
- [ ] `harness/tool_filter.rs` mechanics (299) +
      `subagent_runner/tool_prep.rs` (344) — 01.3
      (live until middleware owns child/toolkit selection; `tool_prep.rs` also
      contains non-filter prompt helpers that must move first)
- [ ] `ThinkingForwarder` — 02.3
      (streaming reasoning moved to `MessageDelta.reasoning`; tool-arg **argument**
      fragments moved to `ToolCallDelta`/`MessageDelta.tool_call` — `emit_tool_args`
      removed. Still live for the tool-call **start** marker `note_tool_call`
      (crate `ToolDelta` has no `tool_name`) and the non-streaming reasoning
      fallback; both must move before deletion)
- [ ] `inference/provider/reliable.rs` (now 900, was 1215 + 1443 tests) — 02.2
      PARTIAL: the shared classifier/backoff exports (`is_non_retryable`,
      `is_rate_limited`, `is_upstream_unhealthy`, `parse_retry_after_ms`,
      `structured_http_4xx`, `compute_backoff`, etc.) + their unit tests were
      extracted to `inference/provider/error_classify.rs`; cross-module importers
      (model.rs, memory_tree, triage, config_rejection, channels) repointed;
      classifier + reliable + triage + tinyagents suites green (138 passed).
      RESIDUAL (the actual delete, deliberately NOT forced): `ReliableProvider`
      is the universal retry/backoff/**model-fallback**/API-key-rotation/
      **streaming-failover** wrapper applied to every provider in
      `provider_factory.rs` and used by non-turn callers (memory-tree, channels)
      that never enter the crate harness. The crate `FallbackPolicy` carries only
      the hardcoded tier map, not user `config.reliability.model_fallbacks`, so
      un-wrapping the turn path would silently drop user fallbacks, and a faithful
      non-turn replacement reconstitutes most of the struct (net-zero) — the only
      true removal is routing all non-turn provider calls through the crate
      harness (large, and unverifiable by the mock suite, which never exercises
      real transient-network retry). Its own header still says "do not delete
      yet." Gated on that re-architecture.
- [x] `tinyagents/orchestration.rs::run_parallel_fanout` — 08.1
- [x] `harness/engine/` (entire dir, 309) — 05.2
- [ ] `agent/progress_tracing.rs` + tests (1338, if duplicate) — 05.2
      (live web-progress exporter until journal-backed projection reaches parity)
- [ ] `harness/run_queue/` mechanics (317 total; 174 non-test) — 07.3
      (live adapter for detached steer/collect plus web followup/parallel;
      split before deleting)
- [ ] crate-internal `harness/spawn_depth_context.rs` (66) — 07.3
      (live recursion guard for nested delegation)
- [x] `harness/worktree_context.rs` (74) — 08.5
      (deleted: worktree-isolated workers now resolve CWD from the carried
      `WorkspaceDescriptor` on `ToolExecutionContext` via
      `effective_action_dir_for_context`; the subagent runner sets the descriptor
      on the worker `RunContext` instead of the task-local. Parity tests
      `shell_uses_workspace_descriptor_root_as_cwd` /
      `git_resolves_cwd_from_workspace_descriptor` assert descriptor→worktree
      root and no-descriptor→`security.action_dir`)
- [x] 32 × `agent_registry/agents/*/graph.rs` stubs (~420) + five default-only
      non-registry graph modules — 08.4
- [x] `tinyagents/checkpoint.rs` (`SqlRunLedgerCheckpointer`, 250) — 04.3
      (deleted: durable delegation graphs now checkpoint through the crate
      `SqliteCheckpointer` at a dedicated `{workspace}/graph_checkpoints.db`.
      Nothing outside the adapter read the old run-ledger `graph_checkpoints`
      table, so the DDL was removed and pre-swap rows simply expire — orphaned
      in-flight tasks are reconciled at boot per 07.2. Delegation + 08.3 durable
      interrupt/resume + session_db suites green: 18 + 43 passed)
- [ ] crate-internal `CostBudgetMiddleware` (`tinyagents/middleware.rs`) +
      crate-internal `agent/harness/turn_subagent_usage.rs` (176) task-local — 06
      (live until crate budget/run-tree accounting avoids duplicate
      `UsageRecorded` and covers parent-turn rollups)
- [ ] `agent/dispatcher.rs` (609) + `harness/parse.rs` (833) legacy tool-call
      parsing — after XML/P-format transcripts read from the store and no
      live path parses provider text (04.2 + verify)
      (live compatibility shell for prompt dialect selection, history
      serialization, XML/P-format fallback parsing, checkpoint cleanup, native
      text fallback, and TinyAgents text-mode provider responses; trim
      unused/test-only parse helpers before full delete)

## Deletable after session-store cutover (04.2 phase 4)

- [ ] `session/transcript.rs` (1347) + tests (978)
- [ ] `session/migration.rs` (373) + tests
- [ ] `session/turn/session_io.rs` (391)
- [ ] `session_db/{ops,store,schemas,types}.rs` generic parts (~1.6k)
- [ ] `agent_orchestration/subagent_sessions/` (~650)
- [ ] `src/openhuman/session_import/` (~1.7k) + RPC controller — one
      release after auto-import ships

## Shrink (not full delete)

- Deleted: `session/agent_tool_exec.rs` 471-line test-only parity shim — 01.4
- `session/turn/tools.rs` 697 → parent-context/assembly glue — 01.3
  (dynamic delegation refresh and skill-event catalogue reconciliation remain
  live until middleware owns contextual selection)
- `subagent_runner/ops/*` 2764 (+1827 companion tests) → graph nodes/tests — 07.1
- `running_subagents.rs` 1250 → ≤~300 policy/executor glue — 07.2
- `tools/spawn_parallel_agents.rs` is a thin tool shell; remaining shrink
  target is `agent_orchestration/spawn_parallel_graph.rs` graph mechanics
  (1280) — 08.2
- `context/` → stats + product prompt state — 03
- `cost/catalog.rs` (622) → catalog snapshot loader once config seeding, cost
  estimates, and `context_window_for_model` all read one catalog projection —
  02.4

## Never delete (product policy)

Prompts/`agent/prompts/`, agent registry definitions, security/approval
semantics, credentials/factory/router names, triage, task board/dispatcher,
archivist, memory stores, worktree policy, `host_runtime.rs`, multimodal
policy, JSON-RPC shapes, `DomainEvent` until all subscribers move.
