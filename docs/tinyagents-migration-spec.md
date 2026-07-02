# TinyAgents Migration Spec

Status: draft migration backlog

TinyAgents source reviewed: `tinyhumansai/tinyagents` `origin/main` at
`8f226f1`, crate version `1.1.0`.

OpenHuman already depends on `tinyagents = "1.1"` and already routes the live
agent turn through `src/openhuman/tinyagents/`. This spec is not a proposal to
add TinyAgents. It is a todo list for moving the rest of OpenHuman's generic
agent runtime behavior onto TinyAgents primitives while keeping OpenHuman-owned
product semantics in OpenHuman.

## Goal

Use TinyAgents as the generic runtime for:

- model/provider abstraction and model selection
- tools and tool schemas
- middleware around model/tool calls
- streaming, events, traces, and replayable run status
- token usage and cost rollups
- state graphs, fanout, reducers, checkpoints, and interrupts
- sub-agent recursion, steering, cancellation, and reusable sessions
- deterministic testkit coverage for the runtime seams

OpenHuman should continue to own:

- desktop product UX and Tauri/RPC boundaries
- user/workspace config, credentials, keychain, and approval records
- OpenHuman memory stores, thread transcripts, run ledgers, and controllers
- security policy, sandboxing, tool permission tiers, and workspace roots
- product-specific built-in agents, prompts, MCP setup, Composio, channels, and
  native tools
- compatibility with existing JSON-RPC method names and persisted state

## Sources Reviewed

TinyAgents SDK:

- `src/lib.rs`
- `src/harness/*`
- `src/graph/*`
- `src/registry/*`
- `src/language/*`
- `src/repl/*`
- `docs/modules/harness/*.md`
- `docs/modules/graph/*.md`
- `docs/modules/registry/*.md`
- `docs/modules/expressive-language/README.md`
- `docs/modules/repl-language/README.md`
- examples: `agent_loop_tools`, `orchestrator_subagents`, `durable_graph`,
  `openai_graph_agent`, `openai_self_blueprint`

OpenHuman Rust core:

- `src/openhuman/tinyagents/*`
- `src/openhuman/agent/**`
- `src/openhuman/agent_orchestration/**`
- `src/openhuman/agent_registry/**`
- `src/openhuman/tools/**`
- `src/openhuman/inference/**`
- `src/openhuman/cost/**`
- `src/openhuman/context/**`
- `src/openhuman/approval/**`
- `src/openhuman/security/**`
- `src/openhuman/mcp_registry/**`
- `src/core/event_bus/**`
- `src/core/all.rs`
- `gitbooks/developing/architecture/agent-harness.md`

## Current Adoption Inventory

Already done or partially done:

- `Cargo.toml` pins `tinyagents = "1.1"` with default features only.
- `src/openhuman/tinyagents/mod.rs` registers OpenHuman `Provider` and `Tool`
  adapters on `tinyagents::harness::runtime::AgentHarness`.
- `ProviderModel` maps OpenHuman `ChatRequest`/`ChatResponse` into
  `tinyagents::harness::model::{ModelRequest, ModelResponse, ModelStream}`.
- `ToolAdapter` and `SharedToolAdapter` map OpenHuman tools into
  `tinyagents::harness::tool::Tool`.
- `OpenhumanEventBridge` maps TinyAgents `AgentEvent` into `AgentProgress` and
  the global cost tracker.
- `StopHookMiddleware`, `ContextCompressionMiddleware`, and
  `MessageTrimMiddleware` are already used on the TinyAgents path.
- `SqlRunLedgerCheckpointer` implements TinyAgents `Checkpointer` on top of the
  OpenHuman session DB because TinyAgents' `sqlite` feature conflicts with the
  current `rusqlite` native-link version.
- `run_parallel_fanout` uses `GraphBuilder`, reducers, command routing, and a
  fan-in barrier for reusable concurrent fanout.
- `model_council`, `workflow_runs`, `agent_teams`, and
  `tinyagents/delegation.rs` already use TinyAgents graphs.
- Built-in agents already have `graph.rs` selectors, but most return
  `AgentGraph::Default`.

Important current gaps:

- OpenHuman still has separate registries for agents, tools, MCP tools, model
  providers, controllers, cost, and event bus projections.
- Tool safety metadata exists in OpenHuman traits but is not fully expressed as
  TinyAgents tool safety/runtime metadata.
- Cost/usage is still converted through a bridge, not an end-to-end TinyAgents
  usage/cost journal.
- Event streams are mirrored into `AgentProgress`, but TinyAgents event journals
  and status stores are not the canonical durable inspection surface.
- Provider model profiles, model resolution, and fallback remain primarily in
  OpenHuman inference/router logic.
- Sub-agent lifecycle, durable state, worker threads, and wait/abort controls
  still live in OpenHuman orchestration stores.

Some todos below are local adapter work. Others require upstream TinyAgents SDK
extensions first. In particular, the SDK has a strong tool/runtime boundary
today (`ToolSchema`, `ToolExecutionContext`, middleware hooks), but OpenHuman's
full tool safety metadata is richer than the current SDK schema. Durable task
storage is another SDK gap: TinyAgents exposes an `InMemoryTaskStore`, while
OpenHuman needs restart-safe SQL/JSON ledgers.

## Migration Rules

- Keep every product-facing JSON-RPC contract stable unless a migration plan is
  written next to the code change.
- Do not bypass OpenHuman approval, security policy, sandbox, workspace root, or
  credential boundaries by adopting a generic TinyAgents tool API.
- Prefer adapters first, then flip ownership once tests prove parity.
- Preserve existing transcript and run-ledger compatibility. TinyAgents may
  become the internal runtime without changing persisted public records in the
  same PR.
- Every migration task needs unit coverage plus at least one JSON-RPC or
  harness-level e2e when behavior crosses controller, tool, provider, or graph
  boundaries.

## Phase 0 - Baseline And Drift Control

- [x] Add a version/feature compatibility note to the OpenHuman architecture doc.
  - OpenHuman files: `gitbooks/developing/architecture/agent-harness.md`,
    `Cargo.toml`.
  - TinyAgents components: crate features `default`, `openai`, `sqlite`, `repl`.
  - Acceptance: document why default features are used, why TinyAgents `sqlite`
    is disabled, and which OpenHuman adapters replace feature-gated SDK
    providers.
  - **Done:** added "TinyAgents crate: features & compatibility" section to
    agent-harness.md (default-only, `openai`/`sqlite`/`repl` rationale, adapter
    map) + fixed stale `council_graph.rs`/`member_graph.rs` links.

## Phase 1 - Tools

- [~] Make OpenHuman tool metadata round-trip into TinyAgents tool metadata.
  - OpenHuman files: `src/openhuman/tools/traits.rs`,
    `src/openhuman/tinyagents/tools.rs`, `src/openhuman/tinyagents/convert.rs`.
  - TinyAgents components: `harness::tool::{ToolSchema, ToolFormat,
    ToolExecutionContext, ToolResult}`.
  - Migrate: permission level, external effect, generated runtime context,
    timeout policy, concurrency safety, result-size cap, display label/detail,
    markdown support.
  - Acceptance: a TinyAgents tool call has enough metadata for middleware to
    enforce approval, security, timeout, concurrency, truncation, and display
    behavior without re-querying OpenHuman trait methods ad hoc.
  - **SDK gap + side-lookup adapter:** crate `ToolSchema` carries only
    name/description/parameters/format — it has **no** metadata/extension map and
    `ToolExecutionContext` is run-scoped, so none of OpenHuman's safety/runtime
    fields have a crate home. The adopted pattern is a shared
    `name → Arc<dyn Tool>` lookup the runner builds from `tool_sets`; middleware
    calls the (often args-aware) trait methods live. Landed uses of it:
    `ApprovalSecurityMiddleware` reads `external_effect_with_args`;
    `ToolOutputMiddleware` now honors each tool's own `max_result_size_chars()`
    (was a flat hardcoded budget). Remaining fields (timeout, concurrency,
    display, generated context) can ride the same lookup as their middlewares
    land; full crate round-trip is blocked pending an SDK `ToolSchema` extension
    map.

- [x] Move unknown-tool recovery into a reusable middleware or tool policy layer.
  - Current shim: `UNKNOWN_TOOL_SENTINEL` in `src/openhuman/tinyagents/tools.rs`.
  - TinyAgents components: `ToolRegistry`, `ToolMiddleware`,
    `AgentEvent::ToolStarted/ToolCompleted`, repairable tool results.
  - Acceptance: hallucinated tool names remain recoverable, sub-agent wording is
    preserved, and TinyAgents event stream records the original requested tool
    name without exposing the sentinel as a model-visible tool.
  - **Done:** `UnknownToolRewriteMiddleware` (`before_tool`) rewrites a call to
    an unadvertised tool onto the sentinel at the tool boundary, before the
    harness resolves it — removing the `valid_tools` plumbing from `ProviderModel`
    (`with_valid_tools`, the field, and the `response_to_model_response` rewrite
    all deleted). Sub-agent vs top-level wording is preserved by the sentinel
    handler; the sentinel is still never advertised. SDK gaps: no
    "tool-not-found → repair" hook (the sentinel handler must stay), and no
    dedicated unknown-tool `AgentEvent` variant (recording the original name in
    the crate event stream would need a manual `ToolStarted` — deferred).

- [x] Route approval and security through TinyAgents middleware.
  - Current OpenHuman files: `src/openhuman/approval/*`,
    `src/openhuman/security/*`, `src/openhuman/tinyagents/tools.rs`.
  - TinyAgents components: `ToolMiddleware`, `ToolExecutionContext`,
    tool safety metadata.
  - Acceptance: approval checks happen in `before_tool`/`wrap_tool`, emit typed
    events, preserve audit rows, and return model-consumable denial results.
  - **Done:** `ApprovalSecurityMiddleware` (`tinyagents/middleware.rs`, a
    `wrap_tool` middleware) replaces the inline approval block in
    `execute_openhuman_tool`. Denials short-circuit with a model-consumable
    result; approved external-effect calls now record a terminal audit row
    (`record_execution`) the old path dropped. Typed approval events still ride
    `DomainEvent` (the crate `AgentEvent` enum has no approval variant — SDK gap).
    Tool-*internal* security (path/command `live_policy`) stays per-tool by
    design. Follow-ups: channel permission-ceiling threading; per-tool metadata
    side-lookup (Task C).

- [ ] Use TinyAgents bounded-concurrent tool execution where safe.
  - OpenHuman files: `src/openhuman/tools/traits.rs`,
    `src/openhuman/tinyagents/tools.rs`.
  - TinyAgents components: graph `Send`, graph fanout, or harness tool
    execution policy.
  - Acceptance: read-only/concurrency-safe tool batches can run in parallel
    with deterministic result ordering and identical transcript semantics.

## Phase 2 - Models And Providers

- [ ] Register OpenHuman inference providers as TinyAgents model registry entries.
  - OpenHuman files: `src/openhuman/inference/provider/*`,
    `src/openhuman/inference/model_ids.rs`, `src/openhuman/tinyagents/model.rs`.
  - TinyAgents components: `harness::model::{ChatModel, ModelRegistry,
    ModelProfile, CapabilitySet, ModelRequest, ModelResponse}`.
  - Acceptance: every workload route (`agentic`, `reasoning`, `coding`,
    `memory`, `subconscious`, etc.) can resolve to a TinyAgents model entry
    while retaining OpenHuman provider strings and config compatibility.

- [ ] Translate OpenHuman provider capability data into TinyAgents model profiles.
  - OpenHuman files: `src/openhuman/inference/provider/traits.rs`,
    `src/openhuman/inference/provider/factory.rs`,
    `docs/inference-provider-catalog.md`.
  - TinyAgents components: `ModelProfile`, `CapabilitySet`, registry model
    catalog.
  - Acceptance: context window, tool calling, streaming, vision, structured
    output, reasoning, local/cloud source, and provider-family metadata are
    available before dispatch.

- [ ] Move model fallback and retry policy to TinyAgents policy/middleware.
  - OpenHuman files: `src/openhuman/inference/provider/reliable.rs`,
    `src/openhuman/inference/provider/router.rs`,
    `src/openhuman/tinyagents/mod.rs`.
  - TinyAgents components: `RunPolicy`, `RetryPolicy`, `FallbackPolicy`,
    `ModelFallbackMiddleware`, `AgentEvent::RetryScheduled`,
    `AgentEvent::FallbackSelected`.
  - Acceptance: OpenHuman provider retry does not double-retry under
    TinyAgents, fallback events are typed, and tests cover transient 429/5xx,
    config rejection, and billing exhaustion.

- [ ] Preserve provider-specific metadata in the TinyAgents message model.
  - OpenHuman files: `src/openhuman/inference/provider/traits.rs`,
    `src/openhuman/tinyagents/convert.rs`,
    `src/openhuman/tinyagents/model.rs`.
  - TinyAgents components: `ContentBlock`, `AssistantMessage`, provider
    metadata, tool-call ids.
  - Acceptance: Gemini thought signatures, reasoning content, native tool-call
    ids, cached tokens, and raw provider metadata survive multi-turn history.

## Phase 3 - Middleware

- [ ] Convert OpenHuman turn cross-cuts into named TinyAgents middleware.
  - Current OpenHuman surfaces: stop hooks, approval gate, security policy,
    output caps, context compression, memory injection, tool allowlists,
    cost/usage, prompt cache stability, event bridge.
  - TinyAgents components: `Middleware`, `ModelMiddleware`, `ToolMiddleware`,
    `MiddlewareStack`, `RunContext`.
  - Acceptance: each cross-cut has a stable middleware name, tests for ordering,
    emitted events, and explicit interaction with streaming/retry/fallback.

- [ ] Add OpenHuman policy middleware for dynamic tool exposure.
  - OpenHuman files: `src/openhuman/agent_registry/*`,
    `src/openhuman/agent/harness/subagent_runner/**`,
    `src/openhuman/tools/user_filter.rs`.
  - TinyAgents components: `before_model`, `before_tool`, `ToolRegistry`.
  - Acceptance: agent `tool_allowlist`, `tool_denylist`, sub-agent tool scope,
    MCP tool visibility, and channel permission ceilings are enforced through
    middleware rather than scattered pre-filtering.

- [ ] Add prompt/cache-layout middleware tests.
  - OpenHuman files: `src/openhuman/context/*`,
    `src/openhuman/agent/harness/session/turn/core.rs`,
    `src/openhuman/tinyagents/summarize.rs`.
  - TinyAgents components: `CachePolicy`, `CacheLayoutEvent`,
    `ContextCompressionMiddleware`, `MessageTrimMiddleware`.
  - Acceptance: system prompt prefix remains stable across later turns; volatile
    memory, timestamps, tool results, and steering messages land in the tail.

## Phase 4 - Events, Status, And Observability

- [ ] Make TinyAgents event journals the canonical internal run event stream.
  - OpenHuman files: `src/openhuman/tinyagents/observability.rs`,
    `src/core/event_bus/*`, `src/openhuman/notifications/*`,
    `src/openhuman/session_db/run_ledger/*`.
  - TinyAgents components: `HarnessEventJournal`, `HarnessStatusStore`,
    `GraphEventJournal`, `GraphStatusStore`, `AgentEvent`, `GraphEvent`.
  - Acceptance: UIs can reconstruct a running or completed agent turn from
    persisted TinyAgents events without relying only on transient
    `AgentProgress`.

- [ ] Bridge TinyAgents events into `DomainEvent` as a compatibility projection.
  - OpenHuman files: `src/core/event_bus/events.rs`,
    `src/openhuman/agent/bus.rs`, `src/openhuman/tinyagents/observability.rs`.
  - Acceptance: existing subscribers continue to receive `DomainEvent`, but new
    code reads TinyAgents events/status first.

- [ ] Persist graph run status and checkpoint metadata in OpenHuman run ledger.
  - OpenHuman files: `src/openhuman/tinyagents/checkpoint.rs`,
    `src/openhuman/session_db/run_ledger/store.rs`,
    `src/openhuman/agent_orchestration/**`.
  - TinyAgents components: `Checkpoint`, `CheckpointMetadata`,
    `GraphRunStatus`, `GraphObservation`.
  - Acceptance: command center, workflow runs, delegation, and team runs can
    list checkpoints, current node/task status, and replay offsets from one DB
    source.

- [~] Export graph topology for debugging and UI inspection.
  - OpenHuman files: built-in `graph.rs` files, `agent_orchestration/*/graph.rs`,
    `model_council/graph.rs`.
  - TinyAgents components: `GraphTopology`, `to_json`, `to_mermaid`,
    validation report.
  - Acceptance: every custom OpenHuman graph has a debug endpoint or test
    snapshot that exports topology and validates missing nodes/routes.
  - **Foundation landed:** new `tinyagents/topology.rs` — `GraphTopologyReport`
    (mermaid + JSON + validation errors/warnings), `describe()`, and
    `all_graph_topologies()`. Pattern: each graph exposes a `build_*_graph`
    (structure) reused by both the runner and a `*_topology()` that builds it
    with no-op stub closures and returns `CompiledGraph::topology()`. Done for
    `agent_teams:member`. Follow-ups (same pattern): `delegation` (injected
    `run_stage` — clean) and `workflow_runs` scheduler (needs a small refactor —
    its node closures capture engine locals). Fan-outs (council,
    `run_parallel_fanout`) are the dispatch→N→collect pattern, not a fixed
    topology. Debug endpoint = call `all_graph_topologies()` (RPC wiring is a
    thin follow-up).

## Phase 5 - Usage, Cost, And Budgets

- [~] Replace bridge-only usage accounting with TinyAgents usage records.
  - OpenHuman files: `src/openhuman/cost/*`,
    `src/openhuman/tinyagents/observability.rs`,
    `src/openhuman/inference/provider/traits.rs`.
  - TinyAgents components: `harness::usage::{Usage, UsageTotals}`,
    `harness::cost::CostTotals`, `AgentEvent::UsageRecorded`.
  - Acceptance: input, output, cached input, reasoning, image/audio, embedding,
    tool/model call counts, and estimated/provider-reported source are recorded
    in normalized records.
  - **Partial (real bug fixed):** the bridge hardcoded `charged_amount_usd: 0.0`
    and `ProviderModel` dropped cached tokens, so EVERY tinyagents turn recorded
    **$0 cost**. Now `model.rs` carries `cached_input_tokens` via crate
    `Usage.cache_read_tokens`, and the bridge estimates per-call cost from
    catalogued per-MTok rates (`cost::catalog::estimate_cost_usd`). Remaining:
    reasoning/image/audio/embedding token fields, model/tool call counts, and an
    explicit estimated-vs-provider-charged `cost_source` tag on `TokenUsage`
    (provider-charged preservation needs an out-of-band carry — crate `Usage` has
    no USD field).

- [~] Move budget checks to pre-call TinyAgents middleware.
  - OpenHuman files: `src/openhuman/cost/tracker.rs`,
    `src/openhuman/tinyagents/mod.rs`.
  - TinyAgents components: `RunPolicy`, cost middleware, `before_model`,
    `before_tool`.
  - Acceptance: per-run, per-thread, daily, and monthly budgets can warn or
    fail before spend where enough data exists, then reconcile after provider
    usage is known.
  - **Partial:** `CostBudgetMiddleware` (`before_model`) fails the run before a
    model call when the global daily/monthly budget is already exceeded
    (`CostTracker::check_budget`), and logs on the warning threshold. Self-gating
    on `config.enabled`; previously daily/monthly enforcement was dormant on the
    tinyagents path. Remaining: per-run/per-thread budgets (need new
    `CostConfig` fields + thread-id threading into the runner) and projecting the
    *next* call's cost pre-spend (needs an input-token estimate).

- [ ] Add cost rollup across sub-agents and graphs.
  - OpenHuman files: `src/openhuman/agent_orchestration/**`,
    `src/openhuman/cost/global.rs`.
  - TinyAgents components: run ids, parent/root run lineage,
    `SubAgentStarted`, `SubAgentCompleted`, graph child runs.
  - Acceptance: parent run totals include child agent/model/tool usage without
    double counting dashboard totals.

## Phase 6 - Graph Runtime And Orchestration

- [ ] Convert remaining ad hoc control loops into explicit TinyAgents graphs.
  - Candidate OpenHuman files: `src/openhuman/agent_orchestration/*`,
    `src/openhuman/subconscious/*`, `src/openhuman/cron/*`,
    `src/openhuman/learning/*`, `src/openhuman/tools/ops.rs`.
  - TinyAgents components: `GraphBuilder`, `Command`, conditional routing,
    reducers, `Send`, barriers, recursion policy.
  - Acceptance: every long-running multi-step orchestration has named nodes,
    route tests, recursion bounds, cancellation checks, and topology export.

- [ ] Replace simple fanout helpers with graph `Send` where payload-specific
  fanout matters.
  - Current helper: `src/openhuman/tinyagents/orchestration.rs`.
  - TinyAgents components: `Command::send`, `GraphInput`, `NodeContext::send_arg`.
  - Acceptance: map-reduce style flows can schedule multiple invocations of the
    same node with distinct payloads instead of materializing one node per item.

- [ ] Make `spawn_parallel_agents` a first-class TinyAgents graph tool.
  - Current OpenHuman files:
    `src/openhuman/agent_orchestration/tools/spawn_parallel_agents.rs`,
    `src/openhuman/tinyagents/orchestration.rs`,
    `src/openhuman/agent_orchestration/worktree.rs`,
    `src/openhuman/agent_orchestration/workflow_runs/engine.rs`.
  - Current behavior: validates at least two tasks, checks parent context,
    enforces `max_parallel_tools`, resolves `AgentDefinition`s, enforces the
    parent `subagents.allowlist`, optionally creates per-worker git worktrees,
    fans workers out through `run_parallel_fanout`, collects results in input
    order, emits `DomainEvent` + `AgentProgress`, detects stale parent file
    reads and cross-worker changed-file overlaps, and returns a structured
    `parallel_agents` JSON payload.
  - TinyAgents components: `GraphBuilder`, `Command`, `Send`,
    `NodeContext::send_arg`, `ChannelSet`/reducers, `GraphEventSink`,
    `SubAgentNode` or OpenHuman `run_subagent` adapter, `RecursionPolicy`,
    `RunPolicy`, `CancellationToken`.
  - Required migration shape:
    - `validate` node: parse tasks, enforce min/max count, parent context,
      allowlist, toolkit requirements, and worktree preflight.
    - `dispatch` node: use `Send` to schedule one worker invocation per task
      instead of generating one static worker node per task.
    - `worker` node: run the OpenHuman sub-agent build pipeline with inherited
      model/tool/security policy, optional `worktree_action_dir`, child task id,
      and bounded turn/output budgets.
    - `collect` reducer: aggregate successes, failures, elapsed time,
      iterations, worktree status, changed files, and stale read markers in
      deterministic task order.
    - `finalize` node: emit compatibility `DomainEvent`/`AgentProgress`
      projections, overlap warnings, and the existing JSON result shape.
  - Acceptance: parallel agent runs are checkpointable, cancellable at graph
    boundaries, bounded by parent policy, observable through TinyAgents graph
    events/status, compatible with existing `spawn_parallel_agents` tool
    output, and able to run edit-capable workers in isolated worktrees without
    silently falling back to shared workspace.

- [ ] Define parallel-agent ownership and scheduling policy explicitly.
  - OpenHuman files: `src/openhuman/agent_registry/agents/orchestrator/prompt.md`,
    `src/openhuman/agent_orchestration/tools/spawn_parallel_agents.rs`,
    `src/openhuman/tools/traits.rs`.
  - TinyAgents components: graph route metadata, `RunPolicy`, task metadata,
    tool safety metadata.
  - Required policy:
    - The parent must provide disjoint ownership boundaries for write-capable
      tasks, or the graph rejects/falls back to serial delegation.
    - Read-only workers may share the parent workspace; write-capable workers
      should request `isolation = "worktree"`.
    - Parent and children inherit one root run id for cost/event rollups, but
      each child gets its own task id and optional worker-thread id.
    - A child cannot widen tools, model choice, sandbox mode, trusted roots, or
      budget beyond the parent-granted policy.
    - Cancellation, steering, and wait/collect must be delivered at graph or
      harness safe boundaries only.
  - Acceptance: the orchestrator can ask for parallelism without prompt-only
    conventions; policy violations become structured graph/tool errors.

- [ ] Make per-agent `graph.rs` selectors real customization points.
  - OpenHuman files: `src/openhuman/agent_registry/agents/*/graph.rs`,
    `src/openhuman/agent/harness/agent_graph.rs`.
  - TinyAgents components: `CompiledGraph`, sub-agent nodes, graph testkit.
  - Acceptance: at least three agents get bespoke graphs where useful:
    orchestrator, researcher, and tool_maker are good first candidates.

- [ ] Keep durable orchestration stores OpenHuman-owned until TinyAgents has a
  durable `TaskStore`.
  - Current OpenHuman files: `running_subagents.rs`, `workflow_runs`,
    `agent_teams`, `command_center`, `subagent_sessions`.
  - TinyAgents limitation: `InMemoryTaskStore` is useful for lifecycle shape but
    not sufficient for desktop restart/resume.
  - Acceptance: do not migrate durable SQL/JSON state to TinyAgents in-memory
    task storage. Use TinyAgents task types as an adapter only.

- [ ] Add graph interrupt/resume for human review points.
  - OpenHuman files: `src/openhuman/approval/*`,
    `src/openhuman/agent_orchestration/workflow_runs/*`,
    `src/openhuman/tinyagents/delegation.rs`.
  - TinyAgents components: `Interrupt`, `ResumeTarget`, `Command::resume`,
    checkpoints.
  - Acceptance: approval/review pauses are durable graph interrupts where the
    run can resume from the exact checkpoint after user action.

## Phase 7 - Sub-Agents, Steering, And Recursion

- [ ] Represent `spawn_subagent`, `steer_subagent`, `wait_subagent`, and
  follow-ups as TinyAgents steering commands plus OpenHuman projections.
  - OpenHuman files: `src/openhuman/agent_orchestration/tools.rs`,
    `src/openhuman/agent_orchestration/running_subagents.rs`,
    `src/openhuman/agent/harness/subagent_runner/**`.
  - TinyAgents components: `SteeringCommand`, `SteeringHandle`,
    `SubAgentSession`, `SubAgentTool`, recursion depth events.
  - Acceptance: mid-flight messages are delivered only at safe loop boundaries,
    accepted/rejected steering emits events, and tool/model allowlists can only
    narrow from parent policy.

- [ ] Reconcile OpenHuman spawn depth with TinyAgents recursion policy.
  - OpenHuman files: `src/openhuman/agent/harness/spawn_depth_context.rs`,
    `src/openhuman/agent/harness/subagent_runner/**`.
  - TinyAgents components: `RecursionPolicy`, `RecursionStack`,
    `SubAgentDepth`.
  - Acceptance: there is one authoritative recursion cap and one error shape,
    with compatibility conversion for existing UI/JSON-RPC responses.

- [ ] Keep OpenHuman's sub-agent build pipeline as product logic.
  - Product-owned pieces: agent definition resolution, prompt assembly, memory
    context, worker-thread mirroring, handoff cache, tool filtering, provider
    routing, sandbox scope.
  - TinyAgents components to adopt beneath it: `SubAgentSession`,
    `ToolExecutionContext`, event lineage, cancellation, usage/cost rollup.
  - Acceptance: use TinyAgents for execution and lineage without flattening
    OpenHuman's agent registry semantics into generic SDK defaults.

## Phase 8 - Registry And Capability Catalog

- [ ] Build a `CapabilityRegistry` projection from OpenHuman registries.
  - OpenHuman files: `agent_registry`, `tools`, `mcp_registry`, `inference`,
    `cost`, `approval`, `security`, controller registry in `src/core/all.rs`.
  - TinyAgents components: `CapabilityRegistry`, `ComponentId`,
    `ComponentKind`, model catalog, graph/tool/agent/store/middleware entries.
  - Acceptance: OpenHuman models, tools, agents, graphs, stores, and middleware
    can be looked up through one policy-aware capability projection.

- [ ] Add registry diagnostics for duplicate names and unsafe aliases.
  - OpenHuman files: `src/openhuman/tools/generated.rs`,
    `mcp_registry`, `composio`, `agent_registry`.
  - TinyAgents components: component names, `ToolRegistry`, diagnostics.
  - Acceptance: duplicate tool names, provider-specific aliases, MCP names, and
    generated tool names fail closed before model dispatch.

- [ ] Use TinyAgents model catalog shape for local provider catalog snapshots.
  - OpenHuman files: `docs/inference-provider-catalog.md`,
    `src/openhuman/inference/presets.rs`,
    `src/openhuman/inference/provider/factory.rs`.
  - TinyAgents components: registry model catalog, local snapshots, price and
    context metadata.
  - Acceptance: model picker, router, budget estimator, and capability filter
    read one normalized catalog projection.

## Phase 9 - Memory, Retrieval, Embeddings, And Context

- [ ] Adapt OpenHuman memory/retrieval to TinyAgents retriever interfaces.
  - OpenHuman files: `memory`, `memory_search`, `memory_tree`,
    `agent/memory_loader.rs`, `context/*`.
  - TinyAgents components: `EmbeddingModel`, `Retriever`, `VectorStore`,
    `ScoredDoc`, context events.
  - Acceptance: the agent harness can load retrieval context through a
    TinyAgents retriever facade while OpenHuman stores remain authoritative.

- [ ] Move context compaction provenance into TinyAgents events.
  - OpenHuman files: `context/README.md`, `tinyagents/summarize.rs`,
    `agent/harness/payload_summarizer.rs`.
  - TinyAgents components: `SummaryRecord`, `Compressed` events, cache layout
    events.
  - Acceptance: every summary records source ids, before/after token estimates,
    policy version, and whether stable prompt prefix was preserved.

- [ ] Normalize embedding usage/cost records.
  - OpenHuman files: `embeddings`, `memory_sync`, `cost`.
  - TinyAgents components: embedding usage fields, model catalog pricing.
  - Acceptance: embedding calls contribute usage and cost with provider/model,
    dimensions, vector count, and source.

## Phase 10 - Dead Code And TinyAgents Re-Expression Audit

Do not delete these blindly. Treat this as an audit list for code that is dead,
vestigial, or generic runtime behavior now expressible through TinyAgents
harness/graph primitives. Delete only after call-site search, compatibility
assessment, and migration coverage are complete.

- [ ] Audit `src/openhuman/agent/harness/engine/*`.
  - Current role: surviving seams from the retired in-house turn loops:
    `CheckpointStrategy`, `ProgressReporter`, and `TurnProgress`.
  - TinyAgents expression: `RunPolicy`, `AgentEvent`, `GraphEvent`,
    `EventSink`, `HarnessStatusStore`, cap/stop middleware.
  - Candidate outcome: move max-iteration and progress projection into
    TinyAgents middleware/events, then delete `engine/*` if no product-specific
    compatibility seam remains.

- [ ] Audit `src/openhuman/agent/harness/agent_graph.rs` and built-in
  `agent_registry/agents/*/graph.rs` default selectors.
  - Current role: per-agent graph hook, but most built-ins return
    `AgentGraph::Default`.
  - TinyAgents expression: a registry of `CompiledGraph`/graph factories keyed
    by agent id, with default graph supplied by the runtime and custom graphs
    registered only where they differ.
  - Candidate outcome: replace dozens of boilerplate default `graph.rs` files
    with registry defaults, keeping files only for agents with custom graphs.

- [~] Audit stale architecture references to removed in-house graph/loop code.
  - Current files: `gitbooks/developing/architecture/agent-harness.md`,
    `src/openhuman/context/README.md`.
  - Candidate stale names: `src/openhuman/agent_graph/`, `GraphBlueprint`,
    `run_turn_engine`, `run_tool_call_loop`, `harness/tool_loop.rs`, old
    context summarizer files.
  - TinyAgents expression: link to live `src/openhuman/tinyagents/*`,
    `GraphBuilder`, `AgentHarness`, `ContextCompressionMiddleware`, and
    graph-export/status surfaces.
  - Candidate outcome: move historical details into a short "pre-migration
    history" appendix or remove them from active architecture docs.
  - **Partial:** flagged the `agent-harness.md` `agent_graph`/`GraphBlueprint`
    section as HISTORICAL-removed (strong inline callout pointing at the live
    tinyagents surfaces); fixed `context/README.md` "Used by" line that still
    referenced the deleted `reduce_before_call`/`ProviderSummarizer`/
    `SegmentRecapSummarizer`/`unified_compaction_enabled`. Remaining: a sweep of
    code doc-comments across many `.rs` files that still name `run_turn_engine`,
    `run_tool_call_loop`, `tool_loop.rs` (comments only — no behavior).

- [ ] Audit `src/openhuman/context/{pipeline,guard,microcompact}.rs`.
  - Current role: context stats/session-memory bookkeeping plus older
    compaction concepts; live history reduction moved to
    `ContextCompressionMiddleware` and `MessageTrimMiddleware`.
  - TinyAgents expression: context-window middleware, `Compressed` events,
    cache layout events, `SummaryRecord`, usage/context pressure status.
  - Candidate outcome: keep only stats/session-memory state that remains
    OpenHuman-specific; move compression policy/provenance into TinyAgents
    middleware and delete unused reduction paths.

- [ ] Audit `src/openhuman/agent/harness/payload_summarizer.rs`.
  - Current role: oversized tool-result compression via a `summarizer`
    sub-agent with a local circuit breaker.
  - TinyAgents expression: `ToolMiddleware::after_tool`,
    `ContextCompressionMiddleware`, `SummaryRecord`, tool artifact/result
    compaction events.
  - Candidate outcome: convert to middleware over TinyAgents tool results so
    the summarizer is no longer a separate OpenHuman-only hook.

- [ ] Audit `src/openhuman/agent_orchestration/running_subagents.rs`.
  - Current role: bespoke live-task registry layered on TinyAgents
    `InMemoryTaskStore`, plus watch channels, abort handles, tombstones,
    task/session lookup, wait, steer, and cancel operations.
  - TinyAgents expression: `TaskStore`, `SteeringCommand`, `SteeringHandle`,
    `CancellationToken`, run tree/status store, durable OpenHuman ledger
    projections.
  - Candidate outcome: keep OpenHuman durable session/worker-thread records,
    but collapse transient lifecycle mechanics into TinyAgents task/status
    primitives once they can represent wait/steer/hard-abort needs.

- [ ] Audit `src/openhuman/agent_orchestration/tools/spawn_parallel_agents.rs`
  after the graph-tool migration.
  - Current role: validation, preflight, fanout, worktree setup, event
    projection, overlap detection, result formatting.
  - TinyAgents expression: graph nodes (`validate`, `dispatch`, `worker`,
    `collect`, `finalize`) with `Send` fanout and reducers.
  - Candidate outcome: keep a thin tool wrapper that invokes the graph and
    formats the existing JSON payload; move orchestration mechanics into the
    graph module.

- [ ] Audit hand-rolled `join_all` fanouts that are workflow orchestration, not
  simple IO batching.
  - Candidate files from current search: `src/openhuman/mcp_registry/registry.rs`,
    `src/openhuman/learning/reflection.rs`,
    `src/openhuman/inference/local/service/ollama_admin/diagnostics.rs`.
  - TinyAgents expression: only migrate fanouts that need agent/run lineage,
    checkpointing, policy, cancellation, or graph observability. Leave simple
    independent IO probes as ordinary Rust concurrency.
  - Candidate outcome: document why each fanout stays as `join_all` or move it
    to `run_parallel_fanout` / graph `Send`.

- [ ] Audit tool registry comments and docs that still describe retired
  direct-loop behavior.
  - Current files: `src/openhuman/tools/traits.rs`,
    `src/openhuman/tools/README.md`,
    `src/openhuman/agent/harness/session/turn/tools.rs`.
  - TinyAgents expression: `ToolRegistry`, `ToolMiddleware`, graph tool nodes,
    tool safety metadata, `ToolExecutionContext`.
  - Candidate outcome: update comments to describe the TinyAgents execution
    path and delete references to the retired serial `harness::tool_loop`
    dispatcher once no code path uses it.

## Phase 11 - Testing And Conformance

- [ ] Create a focused parity test matrix for the current TinyAgents route.
  - OpenHuman files: `src/openhuman/tinyagents/tests.rs`,
    `tests/agent_harness_e2e.rs`, `tests/agent_tool_loop_raw_coverage_e2e.rs`.
  - TinyAgents components: `AgentHarness`, `AgentEvent`, `ToolRegistry`,
    `ModelRegistry`, `MessageTrimMiddleware`, `ContextCompressionMiddleware`.
  - Acceptance: chat turn, channel turn, sub-agent turn, unknown tool recovery,
    approval denial, streaming text, streaming tool args, reasoning deltas,
    early-exit pause, model-call cap, and cost footer all have tests on the
    TinyAgents path.

- [ ] Add a TinyAgents adapter inventory test.
  - OpenHuman files: `src/openhuman/tinyagents/mod.rs`,
    `src/openhuman/tinyagents/tests.rs`.
  - Acceptance: one test asserts that the shared runner registers model, tools,
    middleware, event bridge, context compression, stop hooks, and unknown-tool
    sentinel in the intended order.

- [ ] Port behavior clusters to TinyAgents testkit.
  - OpenHuman files: `src/openhuman/agent/harness/*_tests.rs`,
    `tests/agent_*`.
  - TinyAgents components: harness `testkit`, graph `assert_graph`,
    `GraphEventRecorder`, mock models/tools.
  - Acceptance: legacy assertions over loop wording are replaced by assertions
    over TinyAgents events, checkpoints, graph metadata, and final transcript.

- [ ] Add cross-module e2e tests for graph/sub-agent/model/tool composition.
  - Candidate tests: workflow run with child sub-agents, delegation with review
    loop, council fanout, MCP tool call, Composio approval denial, memory
    retrieval plus summarization.
  - Acceptance: tests cover default features, `openai`-feature compatible code
    paths where available, and all-features where dependency constraints allow.

- [ ] Add fuzz-style graph composition tests.
  - TinyAgents components: graph testkit, reducers, command routing, subgraph
    nodes, sub-agent fake nodes.
  - Acceptance: generated small graphs cover direct edges, conditional routes,
    `Send` fanout, joins, interrupts, recursion caps, and checkpoint resume.

## Suggested Execution Order

1. Confirm version/feature compatibility and SDK-extension gaps.
2. Move tool safety/approval/security into TinyAgents middleware.
3. Normalize usage/cost records and parent/child rollups.
4. Persist TinyAgents event/status journals to the OpenHuman run ledger.
5. Build the OpenHuman `CapabilityRegistry` projection.
6. Convert one high-value built-in agent to a bespoke TinyAgents graph.
7. Adapt memory/retrieval/context surfaces where the TinyAgents interfaces are
   a good fit.
8. Audit and remove or re-express dead/vestigial runtime code through
   TinyAgents harness/graph primitives.
9. Finish with parity, adapter-inventory, conformance, e2e, and fuzz-style tests.

## Non-Goals For The First Migration Wave

- Do not enable TinyAgents `sqlite` until the `rusqlite` native-link conflict is
  solved.
- Do not replace OpenHuman's durable run ledgers with TinyAgents
  `InMemoryTaskStore`.
- Do not expose TinyAgents' OpenAI provider directly to product code while
  OpenHuman provider config, credentials, OAuth, and billing classification are
  still the product source of truth.
- Do not remove `DomainEvent` until all existing subscribers have a TinyAgents
  event/status replacement.
