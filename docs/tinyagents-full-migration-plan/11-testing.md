# 11 — Testing & conformance (run last, per user preference)

Tests are deferred to the end of each workstream slice, with this final
consolidation pass.

## Parity matrix (crate route)

Chat turn, channel turn, sub-agent turn, unknown-tool recovery (RunPolicy),
approval denial, streaming text + reasoning + tool-arg deltas, early-exit
pause, model-call cap checkpoint, budget stop, fallback selection,
compression at 90% window, cache hit/miss, steering inject/cancel,
detached spawn→restart→wait, worktree-isolated parallel run.
Extend `src/openhuman/tinyagents/tests.rs`, `tests/agent_harness_e2e.rs`, and
`tests/agent_tool_loop_raw_coverage_e2e.rs`.

Current execution mode for this migration branch: tests are intentionally
deferred while code/docs are still moving quickly. Use docs drift and cargo
checks for small slices; run the behavioral suites in the final conformance
pass before declaring a workstream done.

## Conformance pass result (2026-07-02)

The deferred suites were re-enabled and run end-to-end after the adapter-first
migration slices landed. Outcome:

- The full test target **compiles** again (it had drifted out of compilation
  while tests were deferred — `execute_tool_call`, `context::guard`,
  `agent::cost`, `tree_loader` removals plus new
  `workspace_descriptor`/`deterministic_cacheable`/capability params and the
  `cache_creation_tokens`/`reasoning_tokens` `UsageInfo` fields).
- **Lib suite: green** — `13700 passed; 0 failed; 29 ignored` under the
  canonical `RUST_MIN_STACK=16777216 … --test-threads=1` (the default
  test-thread stack overflows the deep sub-agent dispatch future; the project
  runner and CI pin the larger stack).
- **All integration/e2e binaries: green single-threaded** (parity matrix in
  `agent_harness_e2e` / `agent_tool_loop_raw_coverage_e2e`, plus the raw
  coverage and `json_rpc_e2e` suites).
- Two genuine regressions were caught and fixed during triage: (1)
  `context_window_for_model` routed through `cost::catalog::lookup` began
  shadowing precise pattern-table windows with rounded catalog rows — fixed
  with a boundary-aware substring match and corrected `gpt-4.1{,-mini}` rows to
  `1_047_576`; (2) the new same-family cross-route fallback masked terminal
  provider errors in single-error test doubles — the doubles now fail on every
  route so error propagation is still exercised.
- Behavioral changes intentionally introduced by the migration were re-baselined
  in the assertions (unknown tools now recover through crate
  `UnknownToolPolicy::ReturnToolError` emitting `unknown tool \`X\`` and bound at
  the iteration cap rather than early-halting; multi-route model/middleware
  inventory counts; `MAX_SPAWN_DEPTH` unified to 3; `session_db` RPC count;
  `npm_exec` workspace-aware CWD).

Known **pre-existing** parallel-isolation flakes (green single-threaded, fail
only under `cargo test` default multi-threading because they mutate shared
process state — unrelated to this migration): `composio_set_api_key_validates_
candidate_key_even_when_stored_key_exists`, `resolved_daily_request_limit_
honors_env`, `resolve_sync_interval_honors_per_toolkit_env`,
`openai_codex_models_url_includes_client_version_query`,
`concurrent_acquire_grants_exactly_one_slot`, and the app_state/backend-
validation + `run_subagent_surfaces_provider_errors_and_can_be_cancelled`
timing tests. These need `serial_test`-style guards, tracked separately.

The mandatory deletion-ledger rows remain the follow-up: each gated deletion
(`reliable.rs`, `ThinkingForwarder`, `tool_filter.rs`/`tool_prep.rs`,
`worktree_context.rs`, `CostBudgetMiddleware`, `SqlRunLedgerCheckpointer`, the
session cutover set) can now proceed one row at a time against this green
baseline, re-running the relevant suite after each removal.

## Testkit adoption

Port legacy loop-wording assertions to crate testkit: `MockModel`
(`with_responses`, `with_tool_call`, `call_count`), graph `assert_graph`/
`GraphEventRecorder`/node doubles (`scripted_route_node`,
`interrupting_node`, `subagent_fake_node`), `RecordingListener` for event
assertions.

## Conformance

- Checkpointer: File vs Sqlite same interrupt/resume behavior (04.3).
- TaskStore: InMemory vs Jsonl lifecycle/filter/restart parity (07.2).
- Graph: `Send` fanout order, failure policy, recursion caps, resume
  (08.x); fuzz-style small-graph composition if time allows.

## Known debts to clear here

- Previously flaky detached-orchestrator e2e timeout/hang debt (see 07.2 step
  6): the tight 2s waits are now 15s, but re-run when tests are re-enabled
  before treating that path as proven.
- `cost::init_global` is now idempotent/no-panic; keep future tests isolated
  around process-global state so they do not depend on execution order.
- Coverage gate: ≥80% on changed lines applies when frontend/rust coverage
  lanes are triggered. Docs-only migration-plan markdown does not trigger the
  coverage lane, but code slices still need the normal changed-line coverage
  before PR completion.
