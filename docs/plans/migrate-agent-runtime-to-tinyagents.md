# Migrate the OpenHuman agent runtime to TinyAgents

> **Status, verified 2026-10-07:** live. Tasks 4, 10 and 11 are outstanding.
> See [`README.md`](README.md) for the whole set and how to re-check it.

> Implementation must proceed bottom-up and test-first. Each task removes the
> old path in the same change that switches its consumers. Do not add shims,
> aliases, forwarding modules, or dual live paths.

## Objective

Make `tinyagents-harness` the only agent runtime. Move host-independent helper
and execution code from `crates/openhuman-core/src/agent/` into the appropriate
vendored crate, wire the existing ten OpenHuman host adapters into a new
host-capability-driven harness entry point, switch every live route, and delete
the manual assembly/task-local shells.

The normative ownership and audit matrix are in
`docs/specs/agent-runtime-upstream-boundary.md`.

## Repository and integration order

The repositories are nested and must be landed bottom-up:

1. `vendor/tinyagents/vendor/tinytools` — branch, tests, commit, push, PR to the
   canonical TinyTools upstream.
2. `vendor/tinyagents/vendor/tinyinference` — independent branch/PR to the
   canonical TinyInference upstream.
3. `vendor/tinyagents` — after the two dependency PRs land (or while pinned to
   their review commits), update its nested gitlinks and implement harness,
   registry, and graph changes; PR to canonical TinyAgents upstream.
4. OpenHuman — update `vendor/tinyagents` gitlink, switch host code and routes,
   delete migrated code, then PR to `tinyhumansai/openhuman` against
   `migrate-inference-to-tinyinference` (or its canonical successor if that
   branch has landed).

Create only the superproject worktree. Do not create nested worktrees in any
submodule. Preserve every auto-commit; never squash. A submodule commit must be
reachable from its upstream PR before the parent gitlink is published.

## Task 0: Lock the boundary with failing architecture checks

**Files**

- Add `scripts/ci/check-agent-runtime-boundary.mjs`.
- Add its command to root `package.json` and the appropriate CI Lite area.
- Add/extend TinyAgents dependency-boundary tests under
  `vendor/tinyagents/crates/tinyagents-integration-tests/tests/`.

**RED**

Write checks that fail on the current tree and report exact imports for:

- every `tinyagents_harness::tool_calling` import or module declaration;
- `crate::agent::{dispatcher,pformat}` and the listed `harness::{parse,
  instructions,tool_filter}` paths;
- `SharedToolAdapter`, `ToolAdapter`, `spec_to_schema`,
  `execute_openhuman_tool`, `assemble_turn_harness`, and
  `run_turn_via_tinyagents_shared`;
- production use of the agent task-local accessors listed in the spec;
- OpenHuman `pub use` of types owned by `tinyagents*`, `tinytools*`, or
  `tinyinference*`;
- TinyAgents facade modules or public re-exports that preserve a moved
  TinyTools/TinyInference/TinyAgents path, including `pub use tinytools_agent`
  from `tinyagents-harness`, behavior-free modules whose public surface is only
  forwarded items, and local aliases/wrappers named after moved symbols;
- host-free TinyAgents crates depending on `openhuman` or naming OpenHuman
  domain types.

Allow a checked-in temporary baseline of exact existing violations so this
task can land green; every later task removes entries, and the final task
deletes the baseline. Do not allow wildcards or path-wide exclusions.

**GREEN**

Run `pnpm rust:layout`, the new checker, and the TinyAgents integration test.

## Task 1: Make `tinytools` the sole tool vocabulary

**Repository:** nested TinyTools first, then TinyAgents consumers.

**Files**

- `vendor/tinyagents/vendor/tinytools/crates/tinytools/src/{tool,result,call,
  context,spec}/` and tests.
- `vendor/tinyagents/crates/tinyagents-harness/src/tool/` and registry/agent
  loop tests.
- Later OpenHuman consumers under `crates/openhuman-core/src/tools/`.

**RED**

Add TinyAgents tests proving a registered `Arc<dyn tinytools::Tool>`:

- receives a `tinytools::ToolRunContext` containing workspace, thread id and
  output cap;
- has model-supplied injected values stripped before host injection and schema
  validation;
- preserves content blocks, markdown preference and reported errors;
- honors timeout/cancellation and emits call-id/name/elapsed correlation as a
  harness event without mutating `tinytools::ToolResult`;
- exposes canonical permission/scope/runtime declarations to policy.

**GREEN**

Remove the public duplicate harness `Tool`, `ToolResult`, `ToolPolicy`, and
schema vocabulary. Change `AgentHarness::register_tool` and registries to
accept `Arc<dyn tinytools::Tool>`. Keep correlation in a harness-owned internal
invocation outcome. Update sub-agent tools and registry capabilities to
implement the canonical trait.

Then change OpenHuman tool implementations to implement `tinytools::Tool`
directly and delete `agent/tinyagents/tools.rs` and
`agent/tinyagents/convert.rs`. Do not add an OpenHuman blanket adapter.

**Verify**

```bash
cargo test --manifest-path vendor/tinyagents/vendor/tinytools/Cargo.toml --workspace
cargo test --manifest-path vendor/tinyagents/Cargo.toml -p tinyagents-harness -p tinyagents-registry
cargo check --manifest-path Cargo.toml -p openhuman
pnpm agent:runtime-boundary
```

## Task 2: Import tool-call protocols directly from `tinytools-agent`

**Files**

- `vendor/tinyagents/vendor/tinytools/crates/tinytools-agent/src/`.
- `vendor/tinyagents/crates/tinyagents-harness/Cargo.toml` and every TinyAgents
  direct consumer.
- OpenHuman `Cargo.toml`, `crates/openhuman-core/Cargo.toml`, prompts, model
  adapters, and parsing tests.

**RED**

Port the OpenHuman XML/JSON/P-Format edge-case fixtures to
`tinytools-agent` tests, including positional-schema recovery, prose
preservation, unknown tools, fenced calls, malformed argument recovery, and
tool-cycle pairing.

**GREEN**

Add direct `tinytools-agent` dependencies where used. Replace every
`tinyagents_harness::tool_calling::*` and OpenHuman helper import with
`tinytools_agent::*`/`tinytools_agent::dialect::*`. Delete:

- `vendor/tinyagents/crates/tinyagents-harness/src/tool_calling/mod.rs`;
- `crates/openhuman-core/src/agent/dispatcher.rs` and tests;
- `crates/openhuman-core/src/agent/pformat.rs` and tests;
- `crates/openhuman-core/src/agent/harness/parse.rs` and tests;
- forwarding helpers in `harness/instructions.rs`, `harness/tool_filter.rs`,
  `context/prompt.rs`, and `registry/tools.rs` after direct consumer updates.

Do not preserve the former paths with `pub use`.

Tasks 2a–2d cover the remaining audited helpers with executable RED/GREEN
steps in
[`migrate-agent-runtime-helper-packages.md`](migrate-agent-runtime-helper-packages.md):
context statistics and prompt mechanics, render helpers, multimodal parsing,
and embedding/retriever interfaces. Complete them before Task 3 consumers are
cut over.

## Task 3: Move model metadata and decorators to `tinyinference-llm`

**Repository:** nested TinyInference.

**Files**

- `vendor/tinyagents/vendor/tinyinference/crates/tinyinference-llm/src/model/`
  (split into `types`, `decorators`, and `observer` if necessary), `usage`,
  `stream`, and tests.
- OpenHuman `agent/tinyagents/{model,abort_guard,resolved_route,routes}.rs`.

**RED**

Port tests for:

- max-token clamping and default/profile application;
- resolved provider/model/route identity for sync and stream responses;
- cache-read, cache-creation, reasoning, charged-amount, and context-window
  usage round trips without `raw` metadata;
- stable run/model-call correlation across deltas and terminal response;
- an unfinished stream aborting its producer when dropped;
- observer callbacks firing once on success, failure, cache hit, and fallback.

**GREEN**

Add native typed metadata and generic decorators/observers in
`tinyinference-llm`. Move `MaxTokensModel`, `ProfileOverrideModel`,
`RouteRecordingModel`, and abort-on-drop behavior there. Replace the
OpenHuman raw `openhuman_usage_meta` envelope and FIFO usage side channel with
typed response/usage fields. Keep OpenHuman provider conversion and pricing
policy at the provider/host boundary.

Delete the OpenHuman decorator definitions, `UsageCarryMiddleware`, and the
resolved-route task-local. Consumers import TinyInference types directly.

## Task 4: Make recursive run context explicit

**Repository:** TinyAgents harness and graph.

**Files**

- `vendor/tinyagents/crates/tinyagents-harness/src/context/{types,mod,test}.rs`.
- `vendor/tinyagents/crates/tinyagents-harness/src/tool/types.rs` (or its
  canonical-tool replacement).
- `vendor/tinyagents/crates/tinyagents-harness/src/subagent/`.
- `vendor/tinyagents/crates/tinyagents-graph/src/recursion/` and tests.

**RED**

Test `RunContext::child` for lineage, maximum depth, cancellation, events,
stores, workspace, thread id, output cap, steering, streaming mode, and
metadata inheritance. Add concurrent-child tests proving values cannot bleed
between sibling runs.

**GREEN**

Add `RunLineage` and `RunContext::child` as specified. Pass explicit context to
sub-agent tools and graph nodes. A child must use the same host-driven harness
entry point as its parent.

In OpenHuman, define one owned `OpenHumanRunContext` (near the host bundle, not
in a generic crate) containing origin, progress, hook/sink handles, artifact
scope, attachments, dispatch/recency/fork state, subagent usage and route
observer. Replace reads of:

- `fork_context`, `sandbox_context`, `spawn_depth_context`,
  `task_recency_context`; attachment, dispatch, and usage state now live on
  `OpenHumanRunContext`;

  Remaining `current_parent` reads are intentionally outside registered tool
  dispatch: the legacy orchestration root builders/entrypoints establish their
  own explicit parent before a direct run, while session construction uses its
  own descriptor as the root fallback. Registered root tools receive
  `OpenHumanRunContext` through typed dispatch and do not consult it.
- `progress_sink`, `resolved_route`,
  `thread_context`, `turn_origin`, and `turn_workspace` task-local scopes.

Delete each task-local module once its final caller receives context directly.
Do not put an opaque global map behind the new API.

## Task 5: Activate `HostCapabilities` in `AgentHarness`

**Repository:** TinyAgents harness.

**Files**

- `vendor/tinyagents/crates/tinyagents-harness/src/runtime/{types,mod,test}.rs`.
- Add `src/runtime/agent.rs` (or equivalently focused module) for
  `AgentTurnRequest` and host-driven invocation.
- `src/host/*`, `src/agent_loop/*`, and integration tests.

**RED**

Build recording implementations for all ten traits and assert exact lifecycle
ordering for a terminal answer, tool round, denied tool, model failure,
cancelled turn, and recursive child. Assert absent optional capabilities are
not called and missing required capabilities produce a typed configuration
error before any model call.

**GREEN**

Add the `HostCapabilities` field, `with_host_capabilities`, `invoke_agent`, and
`invoke_agent_stream`. Drive definition/model/context/security/budget/progress/
memory/learning/outcome/experience through the bundle. Preserve lower-level
explicit-model invocation as a separate SDK API; it must not silently fabricate
host capabilities.

Update `HostCapabilities` trait requests only where live invocation proves
context is missing. Keep request/value types serde/std-oriented and free of
OpenHuman dependencies.

## Task 6: Move definitions into `tinyagents-registry`

**Files**

- `vendor/tinyagents/crates/tinyagents-registry/src/definition/` and tests.
- `crates/openhuman-core/src/agent/harness/{definition*,builtin_definitions*,
  agent_graph.rs}`.
- OpenHuman `agent_registry/` and host `definition_registry.rs`.

**RED**

Move fixtures for TOML parsing, built-in/custom precedence, unknown compiled-out
delegates, tier validation, sandbox/tool scopes, prompt sources, and stable
diagnostics into registry tests.

**GREEN**

Make registry definitions the only definition types used by the harness.
OpenHuman maps built-in prompt/profile/config sources into registry inputs and
implements `DefinitionRegistry` over that store. Keep product enablement and
RPC display DTOs in OpenHuman. Delete OpenHuman generic definition types and
thin graph/type re-exports; update all consumers to direct imports.

## Task 7: Move generic graph and delegation lifecycle

**Files**

- `vendor/tinyagents/crates/tinyagents-graph/src/{delegation,orchestration,
  recursion,todos}/` and tests.
- OpenHuman `agent/orchestration/`, `agent/task_board.rs`,
  `agent/task_dispatcher/`, and `agent/tinyagents/{delegation,orchestration,
  topology,todos}.rs`.

**RED**

Port tests for plan/execute/review/finalize, resume/checkpoint, steering,
parallel failure policy, max depth, todo CAS/selection/cadence, and topology
export. Use fake host-driven agents, not OpenHuman types.

**GREEN**

Move lifecycle algorithms and return crate-native events/outcomes. Keep
OpenHuman worktree policy, RPC/controllers, delivery, workflow product rules,
and durable product ledger projection. Delete thin delegation/orchestration/
todo wrappers and import graph APIs directly. Relocate the legacy task-board
migration to an OpenHuman startup migration module rather than leaving a
facade solely to host it.

## Task 8: Absorb generic middleware into the harness

Implement this as small vertical commits. For every middleware: first port its
behavioral tests to TinyAgents, then move the implementation, then switch and
delete the OpenHuman copy. Keep policy supplied through host traits, typed
configuration, or callbacks.

### 8a. Argument recovery and structured output

- Move `arg_recovery`, required-output extraction/repair, relaxed parsing, and
  final-call wrap-up into existing harness invalid-argument/structured policy.
- Delete OpenHuman `harness/required_output.rs`, `middleware/arg_recovery.rs`,
  and `middleware/final_call_wrap_up.rs` after direct imports.

### 8b. Context trimming, cache layout, and summarization

- Move image-aware trim, prompt-cache layout guard, context ladder,
  payload-summarizer contract/orchestration, and generic compression policy.
- Keep OpenHuman model resolver/config and artifact/memory callbacks.
- Prove tool-call/result pairing, images, cache prefixes, token budgets,
  compaction provenance, and summarizer failure fallback.

### 8c. No-progress and repeated failures

- Fold adjacent repeats, run-wide recurrence, eviction reset, repeated tool
  failure, and terminal inference failure into `tinyagents_harness::no_progress`
  and run policy.
- Test recurrence identity, compaction eviction, successful repeats,
  thresholds, and child-run isolation.

### 8d. Output shaping, redaction, and artifacts

- Move generic recursive JSON/text redaction traversal, size limiting,
  summarization decision, trusted-verbatim handling, handoff storage mechanics,
  and artifact-index/TOC injection.
- Inject host redactor/artifact store callbacks. Keep action-dir authorization,
  credential sources, and OpenHuman artifact persistence in the host.
- Test nested secrets, malformed/large values, byte limits, markdown, exact
  artifact links, and absent-store behavior.

### 8e. Host-policy middleware

- Replace approval, CLI/RPC-only, cost, memory, exposure, outcome, hooks and
  policy middleware mechanics with calls made by host-capability invocation.
- Keep only OpenHuman trait implementations and denial/audit persistence.
- Confirm authorization occurs after canonical argument injection and before
  execution, and denial cannot be bypassed through aliases/packed calls.

### 8f. Remaining harness helpers

- Replace OpenHuman `harness/run_queue/` with direct
  `tinyagents_harness::run_queue` use after moving its remaining parity tests.
- Move host-free archivist lifecycle/recap heuristics behind upstream memory
  and store traits; keep OpenHuman memory-tree persistence, product events and
  policy in the host adapter.
- Move artifact-offload sizing/chunking/handoff/extraction mechanics, including
  `ResultHandoffCache`, into `tinyagents-harness::handoff`; inject the
  OpenHuman action-dir artifact store and authorization callback.
- Keep `tool_result_artifacts/` as the host store implementation, not as an
  upstream re-export facade.
- Port cache bounds, chunk reconstruction, authorization, missing/expired
  handles, recap ordering and queue fairness tests before deleting each host
  helper.

## Task 9: Build and test the OpenHuman host bundle

**Files**

- `crates/openhuman-core/src/agent/tinyagents/host/mod.rs` and ten adapter files.
- Add focused integration tests beside the bundle or under root `tests/` with
  explicit `[[test]]` entries when new root test files are created.

**RED**

Write a single end-to-end harness test that constructs the real OpenHuman
bundle with fake domain backends and proves all ten adapters are reached by a
live `invoke_agent` turn. Add security regressions for always-forbidden paths,
action-dir escape, unknown command classification, approval default/expiry,
untrusted input screening, memory scope/self-echo, and credential redaction.

**GREEN**

Add one `build_host_capabilities(...) -> HostCapabilities<_>` construction
site. Resolve existing `TODO(phase4)` gaps with real domain calls or explicitly
omit an optional capability; never register an erroring placeholder. Move
`turn_models.rs`, host config mapping, and policy-denial persistence beside
their adapters. Remove test-only accessors that construct individual adapters.

## Task 10: Switch every live route to host-driven invocation

**Files/callers**

- `agent/harness/session/turn/graph.rs` and session runtime.
- `agent/bus.rs`, channel/CLI paths, cron agent jobs, task dispatcher, triage,
  sub-agent tools, workflow runs, web chat, and embed harness.
- `crates/openhuman-embed/` public prompt-to-reply path.

**RED**

For each route, add a test asserting the request reaches
`invoke_agent[_stream]` with agent id, crate-native messages, explicit
`OpenHumanRunContext`, turn origin/access, workspace/config path, and inherited
cancellation. Add parity tests for streaming, early exit, steering, tool
timeline, terminal progress, error mapping, usage/cost, and child events.

**GREEN**

Switch all routes. Keep durable `ChatMessage` conversion only where reading or
writing existing OpenHuman storage/import/export contracts. `openhuman_embed::Harness`
must still enter through `CoreRuntime::invoke`, use one harness per process,
and set workspace/config/origin correctly.

There is no runtime flag or fallback to `run_turn_via_tinyagents_shared`.

## Task 11: Delete the OpenHuman runtime shell and direct-import all owners

**Delete after `rg` proves no consumers**

- `agent/tinyagents/{harness_assembly,harness_context_ladder,
  harness_tool_registration,turn_runner,convert,tools,abort_guard}.rs`;
- migrated `model`, `routes`, `resolved_route`, middleware, summarizer,
  outcome/finalization, task-local, delegation/orchestration/todo files;
- old generic `agent/harness/` files and session/subagent loop code superseded
  by `AgentHarness`;
- `dispatcher.rs`, `pformat.rs`, graph/tool/parse/filter re-export facades;
- broad exports from `agent/tinyagents/mod.rs`, then the module itself if only
  host-owned children remain (relocate those children under clear product
  modules first).

Update README/architecture docs to describe OpenHuman as a host. Remove stale
tests rather than leaving tests of deleted wrappers; their behavior must
already exist at the owning crate or host integration level.

Run the boundary checker with an empty baseline and these explicit searches:

```bash
rg -n "run_turn_via_tinyagents_shared|assemble_turn_harness|SharedToolAdapter|ToolAdapter|spec_to_schema|execute_openhuman_tool" crates vendor/tinyagents/crates
rg -n "tinyagents_harness::tool_calling|crate::agent::(dispatcher|pformat)" crates vendor/tinyagents/crates
rg -n "tokio::task_local!" crates/openhuman-core/src/agent
```

Any remaining task-local requires a documented proof that it is not a policy,
correctness, recursion, cancellation, workspace, or observability input.

## Task 12: Stage, validate, and land bottom-up

The exact non-overlapping ownership waves, handoff gates, full validation
commands, and landing checks are in
[`migrate-agent-runtime-waves.md`](migrate-agent-runtime-waves.md). A later wave
must not begin until its gate is satisfied. In particular, no consumer deletion
may precede a tested upstream owner API, and no OpenHuman cutover may precede a
TinyAgents commit that records its TinyTools and TinyInference gitlinks.
