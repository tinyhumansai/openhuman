# Agent runtime upstream boundary

> **Status, verified 2026-10-07.** The migration this spec governs is mostly
> executed. Its audit matrix is the original work-list, so several rows name
> paths that have since been deleted and several "Delete" rows are already
> done; one Package A target, `crates/openhuman-core/src/agent/registry/tools.rs`,
> is still present. Read
> [`../plans/README.md`](../plans/README.md) for the per-plan status and the
> commands that re-check it, and treat the matrix below as the record of intent
> rather than a to-do list.

**Status:** migration specification
**Scope:** `crates/openhuman-core/src/agent/` and the vendored `tinyagents`,
`tinytools`, and `tinyinference` repositories
**Compatibility policy:** breaking, direct imports only; no compatibility modules,
type aliases, forwarding wrappers, or OpenHuman re-export facades

## Goal

OpenHuman must become a host of the TinyAgents harness rather than a second
agent runtime. Code that describes an agent loop, model/tool protocol,
recursion, middleware mechanics, or graph lifecycle belongs in the smallest
reusable upstream crate. OpenHuman retains product policy, durable product wire
formats, UI/RPC projection, configuration, and adapters that implement the
upstream host traits.

This is a full import migration. A moved API is imported from its owning crate
at every call site. Old paths are deleted in the same work package. There is no
transition period in which both paths compile.

## Non-negotiable boundaries

### Move upstream

- Model-facing tool syntax, parsing, catalogue rendering, transcript pairing,
  and P-Format live in `tinytools-agent`.
- The canonical tool trait, declaration, result blocks, permissions, runtime
  hints, and run-context vocabulary live in `tinytools`.
- Model request/response metadata, usage, resolved route identity, streaming
  correlation, abort-on-drop, and generic `ChatModel` decorators live in
  `tinyinference-llm`.
- The agent loop, host-capability invocation, context inheritance, generic
  middleware, summarization mechanics, handoff mechanics, and generic event
  vocabulary live in `tinyagents-harness`.
- Agent-definition parsing, validation, and named definition lookup live in
  `tinyagents-registry`.
- Delegation, recursion, task/todo selection, and generic graph lifecycle live
  in `tinyagents-graph`.

### Keep in OpenHuman

- The `{role, content}` wire adapter for the two host-owned files that embed
  message rows (`agent::messages::history_wire`: the durable sub-agent session
  store and the sub-agent pause checkpoint). There is no host message type: the
  row is `tinyagents_session::transcript::TranscriptMessage`, typed dialect
  entries are `tinytools_agent::dialect::TranscriptEntry`, and conversion to/from
  `tinyinference_llm::message::Message` stays at the transcript codec boundary;
  live turns use crate-native messages.
- Security policy, approval, origin/taint rules, action-directory enforcement,
  workspace guards, credential handling, and sandbox selection.
- OpenHuman configuration, tier selection, provider credentials, provider
  construction, pricing, charged-cost accounting, and product feature gates.
- Product prompts, profiles, memory, learning, experience, artifacts,
  integrations, channels, triage business rules, and task-board business
  behavior.
- RPC schemas/controllers, `DomainEvent`, `AgentProgress`, frontend timeline
  projection, Langfuse transport, and other product observability exports.
- OpenHuman host implementations of the ten TinyAgents capability traits.
- OpenHuman run journals, legacy import, replay controllers, orphan reaping,
  and migrations whose on-disk layout is an OpenHuman compatibility contract.

Keeping a domain in OpenHuman does not justify keeping generic helper code next
to it. The host should pass product behavior through a trait or callback and
import the generic mechanism directly.

## The current architectural defect

`tinyagents_harness::host::HostCapabilities<State>` already bundles ten seams:
`ContextComposer`, `DefinitionRegistry`, `SecurityGate`, `ModelResolver`,
`AgentMemory`, `BudgetGate`, `ProgressSink`, `LearningSink`,
`ToolOutcomeClassifier`, and `ExperienceStore`. OpenHuman implements all ten in
`agent/tinyagents/host/`.

The bundle is dead infrastructure today. `AgentHarness` has no host-capability
field, no `with_host_capabilities`, and no host-driven `invoke_agent` entry
point. Production instead constructs models, tools, middleware, context, and
side channels manually in `harness_assembly.rs` and `turn_runner.rs`. Merely
moving helpers while leaving that path intact would preserve two runtimes and
is not completion.

The P0 upstream contract is therefore:

```rust
pub struct AgentTurnRequest {
    pub agent_id: String,
    pub messages: Vec<tinyinference_llm::message::Message>,
}

impl<State: Send + Sync, Ctx: Send + Sync> AgentHarness<State, Ctx> {
    pub fn with_host_capabilities(
        &mut self,
        host: HostCapabilities<State>,
    ) -> &mut Self;

    pub async fn invoke_agent(
        &self,
        request: AgentTurnRequest,
        context: RunContext<Ctx>,
        state: &State,
    ) -> Result<AgentRun>;

    pub async fn invoke_agent_stream(
        &self,
        request: AgentTurnRequest,
        context: RunContext<Ctx>,
        state: &State,
    ) -> Result<AgentStream>;
}
```

The exact ownership/borrowing details may follow existing harness conventions,
but these semantics are required:

1. resolve the definition and model through the bundle;
2. compose and screen context before the first model call;
3. authorize every tool call;
4. acquire and record budget around every model call;
5. emit progress without allowing sink failure to fail the turn;
6. recall optional memory/experience only when configured;
7. classify tool outcomes through the optional classifier;
8. notify learning/experience after the terminal outcome;
9. run child agents through the same entry point and inherited context;
10. fail explicitly when the high-level entry point lacks required host
    capabilities. The lower-level model-name invocation may remain for SDK
    users that deliberately assemble a harness themselves.

## Migration packages from the audit

The implementation plan expands these audited packages into test-first tasks;
the labels remain useful for review and completion tracking:

| Package | Required result |
| --- | --- |
| **A — direct imports and facade purge** | Import `tinytools-agent` directly; delete `dispatcher.rs`, `pformat.rs`, `tool_dialect.rs`, production `harness/parse.rs` re-exports, `harness/instructions.rs`, `harness/tool_filter.rs`, `context/prompt.rs`, `registry/tools.rs`, the handoff wrapper, and behavior-free graph re-exports. |
| **B — TinyInference ownership** | Move max-token/default/profile decorators, route identity, typed usage/provider metadata, call correlation, and abort-on-drop stream behavior to `tinyinference-llm`. |
| **C — canonical tool vocabulary** | Use `tinytools::Tool`/`ToolResult` end to end; make the harness bridge canonical tool context into its loop; delete OpenHuman tool adapters and conversions. |
| **D — live host invocation** | Give `AgentHarness` a real `HostCapabilities`-driven entry point, wire all ten OpenHuman adapters, and replace correctness-sensitive task-locals with explicit recursively inherited `RunContext` data. |
| **E — generic middleware absorption** | Move argument recovery through existing invalid-argument policy, trimming, cache layout, required-output repair, no-progress/failure guards, final-call wrap-up, summarizer mechanics, recursive redaction/output shaping, handoff, and artifact-index/TOC mechanics into the harness. |
| **F — route cutover and deletion** | Switch every live route to the host-driven API, delete manual harness assembly and the task-local runtime shell, and leave only product adapters/policy/durable boundaries in OpenHuman. |

Packages are dependency-ordered as C/B, A, D, registry/graph work, E, then F.
No package is complete while its old path or a forwarding replacement remains.

## Canonical cross-crate APIs

### `tinytools` and `tinytools-agent`

`tinytools::Tool`, `tinytools::ToolSpec`, `tinytools::ToolResult`,
`tinytools::ToolCallOptions`, and `tinytools::ToolRunContext` are the sole tool
vocabulary. `tinyagents-harness` must not expose a second public Tool trait or
ToolResult shape. Harness-only correlation (`call_id`, elapsed time, run id)
belongs to an internal invocation record/event, not the tool result.

`AgentHarness::register_tool` accepts `Arc<dyn tinytools::Tool>`. A recursive
canonical tool that needs its typed parent (for example OpenHuman's
`spawn_parallel_agents`) uses `AgentHarness::register_tool_dispatch` and an
explicit `ToolDispatch<State, Ctx>` instead of recovering state from a
task-local. The harness
implements its execution context as `tinytools::ToolRunContext` and performs
injected-argument, timeout, cancellation, and event handling around the
canonical call. OpenHuman tools implement `tinytools::Tool` directly; the
`SharedToolAdapter`, `ToolAdapter`, `spec_to_schema`, and
`execute_openhuman_tool` bridge disappear.

Thread ownership follows the same boundary: a tool reads its caller thread
only from `ToolRunContext::thread_id()`. Recursive and detached sub-agent
launches copy that explicit value into their owned run options before spawning;
they do not re-scope a thread task-local. Memory recall receives its session
and exclusion identity from TinyAgents' `RecallRequest`.

All users import parsing APIs from `tinytools_agent::{...}` or
`tinytools_agent::dialect::{...}`. Delete
`tinyagents_harness::tool_calling`, the former OpenHuman `agent::dispatcher`
and `agent::pformat` facades, and any forwarding definitions. The retained
OpenHuman's persisted transcript and provider types are converted only at their
concrete I/O boundaries; they do not define a dialect facade.

The same direct-import rule applies between TinyAgents crates. A harness,
registry, or graph module must not preserve a former path by publicly
re-exporting a TinyTools, TinyInference, or sibling TinyAgents API. Facades,
type aliases, forwarding traits/functions, and behavior-free wrapper modules
are migration failures even if OpenHuman no longer contains the shim.

### `tinyinference-llm`

The inference crate owns information that is true at a model call boundary:

- `Usage` represents input, output, cache-read, cache-creation, reasoning,
  charged amount, and context-window figures without hiding fields in
  `ModelResponse::raw`.
- request/response/stream events carry stable run/call correlation and resolved
  provider/model/route identity.
- reusable decorators own max-token bounding, profile/default application,
  route observation, usage observation, and abort-on-drop streaming. Their
  callbacks are generic observers; they do not name OpenHuman types.
- OpenHuman maps its provider DTO at the provider boundary and keeps pricing
  policy and tier selection host-side.

The canonical `RouteRecordingModel` stamps this metadata on unary responses
and stream terminal metadata. OpenHuman's typed model middleware copies only
the successful response route into its explicit run context, and the turn
outcome carries it to channel callers. This removes the OpenHuman
`RouteRecordingModel` and resolved-route task-local bridge; the remaining
host wrappers stay only while their product policy is not yet upstream.

### Explicit recursive context

No behavior required by a child run may depend on a Tokio task-local. Extend
`RunConfig`/`RunContext` with explicit lineage and a child constructor:

```rust
pub struct RunLineage {
    pub root_run_id: RunId,
    pub parent_run_id: Option<RunId>,
    pub depth: usize,
}

impl<Ctx> RunContext<Ctx> {
    pub fn child<ChildCtx>(
        &self,
        config: RunConfig,
        data: ChildCtx,
    ) -> Result<RunContext<ChildCtx>>;
}
```

`child` inherits cancellation, events, stores, workspace, thread lineage,
output/depth limits, steering policy, and serializable metadata according to a
documented rule. OpenHuman's `Ctx` contains explicit host handles/values for
origin, progress, stop hooks, artifact scope, attachments, dispatch guard,
recency, fork/cache context, subagent usage, and resolved-route observation.
Sub-agent tools receive it from `ToolExecutionContext`; detached tasks clone
the owned handles explicitly. Task-locals may remain only for process-local
conveniences that are neither correctness nor policy inputs, and the final
forbidden-symbol check lists the old scopes.

### Definitions and graphs

Definition source parsing, validation, tier/delegate validation, and lookup are
`tinyagents-registry` responsibilities. OpenHuman retains built-in prompt
assets, profile persistence, enablement, and conversion into registry inputs.
Callers use registry types directly; OpenHuman does not re-export them.

Generic delegation state machines, recursive run lifecycle, steering registry,
topology export, and todo selection are `tinyagents-graph` responsibilities.
OpenHuman retains workflow RPC/domain decisions, worktree creation policy,
delivery, durable product ledgers, and graph-event projection.

Do not add `tinyagents-inspect` during the initial migration. Diagnostics can
be added as a separate crate later if multiple consumers need a stable
inspection API; creating it now would turn relocation into an API-design
detour.

## Exhaustive audit matrix

The disposition is for production ownership after migration. “Delete” means
all consumers import the owner directly, not that behavior is dropped.

### Top-level `agent/` inventory

| Current path | Disposition | Owner / rationale |
| --- | --- | --- |
| `artifacts/` | Keep | OpenHuman artifact persistence and action-dir policy; move generic artifact-index/TOC middleware mechanics to harness. |
| `context/` | Split | Keep product context/session-memory inputs; move prompt/context mechanics and stats helpers that name no host type to harness. Delete `context/prompt.rs` facade. |
| `debug/` | Keep | Product debug/RPC rendering; import harness diagnostics directly. |
| `experience/` | Keep | Product store, RPC, and prompt data; accessed via `ExperienceStore`. |
| `file_state/` | Keep | OpenHuman parallel-write safety policy; pass explicit context rather than task-local scope. |
| `git_attribution/` | Delete | Obsolete host feature; removed independently before this migration. |
| `harness/` | Split then shrink | Move generic definitions, recursion, parsing, filtering, graph and loop helpers to TinyAgents; keep public OpenHuman session facade only until live callers use the crate harness, then replace it with host construction/routing. |
| `learning/` | Keep | Product learning/profile policy behind `LearningSink`. Generic post-turn callback timing moves to harness. |
| `library/` | Keep | Product-facing safe definition projection/RPC DTO. |
| `orchestration/` | Split | Move generic graph lifecycle/delegation/selection; keep worktrees, workflow business rules, delivery, RPC, ledgers, and host tools. |
| `plan_review/` | Keep | OpenHuman interactive approval policy and RPC state. |
| `profiles/` | Delete | Removed in this migration; agent definitions come from the registry. |
| `progress_tracing.rs`, `progress_tracing/` | Keep | OpenHuman UI/Langfuse projection and transport. Generic observations/events remain upstream. |
| `prompts/` | Keep/Split | Keep bundled product prompts and section content. Move behavior-free render helpers/building blocks that name no product type to harness/prompt. |
| `registry/` | Split | Keep enablement, custom-agent persistence, product defaults and RPC; move definition parsing/validation/lookup to `tinyagents-registry`; delete thin `registry/tools.rs`. |
| `session_db/` | Keep | OpenHuman RPC controllers over TinyAgents ledger. |
| `session_import/` | Keep | Legacy OpenHuman source and disk migration. |
| `task_dispatcher/` | Delete | Upstream removed the task runtime. Delete the obsolete poller, executor, task-board binding and their callers; do not recreate a host shim. |
| `tinyagents/` | Split/Delete | Host adapters and product journal/projections stay; generic assembly, model/tool wrappers, middleware, task-local routing and graph wrappers move upstream, then the integration facade disappears. |
| `tools/` | Keep | Product control tools, changed to implement `tinytools::Tool` directly. Generic todo/delegation mechanics move to graph/harness. |
| `tools.rs` | Keep aggregator | Re-export OpenHuman-owned product tools only; import all upstream types at use sites. |
| `triage/` | Keep | OpenHuman trigger business domain and RPC. It invokes the host-driven harness. |
| `bus.rs` | Keep | Process-wide OpenHuman bus registration. |
| `cost.rs` | Keep/Split | Keep pricing/tier accounting; model-call usage carrier and aggregation primitives move to inference/harness. |
| `tool_dialect.rs` | Delete | Import `tinytools-agent::dialect` directly; keep only concrete I/O conversions. |
| `error.rs` | Keep/Shrink | Product-facing errors and RPC mapping only; loop/model/tool errors originate upstream. |
| `hooks.rs`, `stop_hooks.rs` | Keep adapter | Product hook policy; generic callback/control machinery is harness-owned and values are explicit in context. |
| `host_runtime.rs`, `platform_shell.rs` | Keep | Host execution/shell policy shared with sandbox. |
| `message_convert.rs` | Keep at boundary | Only legacy persistence/import/provider boundaries may convert; remove it from live harness assembly. |
| `messages.rs` | Keep | Durable OpenHuman transcript wire contract. |
| `multimodal.rs` | Split | Keep file access/security and host attachment resolution; import marker/MIME/data-URI mechanics from harness directly, with no re-exports. |
| `pformat.rs` | Delete | Import `tinytools-agent` directly. |
| `progress.rs`, `progress_sink.rs` | Keep adapter | UI contract/sink mapping remain; task-local delivery becomes explicit `RunContext` data. |
| `schemas.rs` | Keep | OpenHuman RPC controllers. |
| `task_board.rs` | Delete facade | Call `tinyagents_graph::todos` directly; keep only product migrations at their owning startup module. |
| `task_session.rs` | Delete | It existed solely for the removed autonomous task-board dispatcher. |
| `tool_policy.rs` | Keep policy | Product restrictions/approval facts; invoke through `SecurityGate`/canonical tool declarations. |
| `turn_origin.rs`, `turn_workspace.rs` | Keep types/delete scopes | Product origin/workspace rules remain, but task-local accessors are replaced by explicit run context. |
| `mod.rs`, `README.md` | Update | Export only OpenHuman-owned public API and describe the host boundary. |
| top-level `*_tests.rs` | Move or rewrite | Generic behavior tests move with their implementation; remaining tests cover OpenHuman adapters, wire formats, policy, and routes. |

### `agent/harness/` generic candidates

| Current area | Destination | Final OpenHuman state |
| --- | --- | --- |
| `definition*`, `builtin_definitions*`, `agent_graph.rs` | `tinyagents-registry` / `tinyagents-graph` | Host supplies product sources; direct imports. |
| `parse.rs`, `instructions.rs`, `tool_filter.rs` | `tinytools-agent` / `tinyagents-harness` | Delete. |
| `graph.rs` and thin graph re-exports | `tinyagents-graph` | Delete facade. |
| `required_output.rs` | `tinyagents-harness::structured` | Delete after parity tests. |
| `memory_protocol.rs`, context ladder helpers | `tinyagents-harness` | Product memory adapter remains. |
| `archivist/` | Split | Move generic lifecycle/recap heuristics behind memory/store traits; keep OpenHuman memory-tree persistence, events, and policy in its adapter. |
| `artifact_offload/` | Split | Move size/chunk/handoff policy mechanics to harness; keep action-dir authorization and OpenHuman artifact store callbacks. |
| `run_queue/` | `tinyagents-harness::run_queue` | Delete the OpenHuman copy/facade after parity tests and direct imports. |
| `tool_result_artifacts/` | Keep host store | OpenHuman action-dir artifact persistence implementing the upstream artifact callback/store seam. |
| `credentials.rs`, `memory_context_safety.rs` | Keep host policy/adapters | Feed explicit context/capability requests; move only host-free formatting/traversal helpers. |
| `fork_context.rs`, `sandbox_context.rs`, `spawn_depth_context.rs`, `task_recency_context.rs` | explicit `RunContext`/child context | Delete task-local shells. |
| `subagent_runner/` | Delete | The complete legacy tree is replaced by direct `agent/subagent_host` adapters over `tinyagents-orchestration::subagent`; generic lifecycle ordering, coalescing and mutually exclusive persistence are upstream. |
| `subagent_host/` | Keep host adapter | OpenHuman implements `SubagentPlanner`, `SubagentExecutor` and `SubagentPersistence`: definitions, policy, provider/model selection, tool narrowing, workspace/security, progress, artifacts and durable product checkpoint/session projection remain host-owned. |
| `subagent_host/handoff.rs`, `extract_tool.rs` | Host callsite / `tinyagents-harness::handoff` | Use `ResultHandoffCache` directly; retain only OpenHuman's configured threshold and product extraction tool. |
| `session/` loop/turn assembly | `AgentHarness::invoke_agent[_stream]` | Keep OpenHuman transcript/checkpoint persistence and a thin product-facing constructor only if it adds a real OpenHuman API; no runtime duplication. |
| credentials, memory, profile, prompt and sandbox inputs | Host adapters | Keep host implementations; no generic mechanics. |

### `agent/tinyagents/` inventory

| Current path | Disposition |
| --- | --- |
| `host/*` | Keep as the ten OpenHuman adapter implementations; wire all into one bundle factory. |
| `harness_assembly.rs`, `harness_context_ladder.rs`, `harness_tool_registration.rs`, `turn_runner.rs` | Delete after host-driven invocation owns assembly and live routes switch. |
| `convert.rs`, `tools.rs` | Delete after canonical `tinytools::Tool` adoption. |
| `model.rs`, `abort_guard.rs`, route/usage helpers in `routes.rs` | Move generic pieces to `tinyinference-llm`; keep tier/fallback choice in host model resolver. Route observation already uses TinyInference response metadata and an explicit OpenHuman run-context field; no task-local slot remains. |
| `middleware/{arg_recovery,message_trim,prompt_cache,repeat_progress,repeated_failure,final_call_wrap_up,artifact_index_toc,credential_scrub}.rs` | Move generic mechanism to harness, parameterized by policy/callbacks. |
| `middleware/{approval,cli_rpc_only,cost_budget,embedder_hooks,memory_protocol,packed_tool_route,tool_exposure,tool_outcome_capture,tool_output,tool_policy,turn_context}.rs` | Split: upstream lifecycle/mechanics; OpenHuman policy and domain adapters remain behind host traits/context. No forwarding middleware module. |
| `middleware/loop_guards.rs` | Move generic terminal/no-progress behavior to harness. |
| `payload_summarizer.rs`, `summarize.rs` | Move generic summarizer orchestration/trait to harness; OpenHuman model selection/config adapter stays. |
| `turn_policy.rs`, `turn_outcome.rs`, `turn_run_error.rs`, `turn_run_finalize.rs` | Move generic run policy/outcome/finalization upstream; retain product error/progress/cost projection only. |
| `turn_models.rs`, remaining `routes.rs` | Keep as `ModelResolver` implementation/config projection, then relocate under the host adapter rather than an integration facade. |
| `thread_context.rs`, `stop_hooks.rs`, `steering_forwarder.rs` | Replace with explicit run/child context; keep only product stop-hook adapter. The former ambient cancellation carrier is deleted: recursive fan-out receives the parent token through typed tool dispatch. |
| `orchestration.rs`, `delegation.rs`, `topology.rs` | Move generic lifecycle/helpers to graph; keep product topology composition only where it genuinely names product graphs. |
| `observability/*` | Keep OpenHuman projection (`AgentProgress`, cost, Langfuse); move reusable graph/event/cap mechanics upstream and import directly. |
| `journal.rs`, `reaper.rs`, `replay/*` | Keep because layout/RPC are product durability contracts; use upstream journal traits directly. |
| `todos.rs` | Delete facade; retain one-shot OpenHuman migration in a product startup/migration module. |
| `embeddings.rs`, `retriever.rs` | Move generic interfaces/adapters upstream where host-free; keep provider/config and memory-domain binding. |
| `config.rs`, `policy_denial.rs` | Keep product mapping/audit persistence, but relocate beside their host adapters; delete the broad `tinyagents/mod.rs` facade. |

## Acceptance criteria

- Every production turn route calls `AgentHarness::invoke_agent` or
  `invoke_agent_stream` with a `HostCapabilities` bundle.
- All ten OpenHuman host adapters have live-path contract tests; none is only
  test-referenced.
- There is one public tool trait/result vocabulary (`tinytools`) and one
  tool-call dialect owner (`tinytools-agent`).
- Live history is crate-native; OpenHuman transcript conversion appears only
  at persistence/import/export boundaries.
- Recursive child runs use `RunContext::child`; no listed agent task-local is
  read for policy, cancellation, workspace, origin, progress, route, or
  recursion state.
- No moved symbol is re-exported from OpenHuman or from a compatibility module
  in TinyAgents, including a facade over TinyTools, TinyInference, or a sibling
  TinyAgents crate.
- `rg` finds none of the deleted modules or adapter names in production code.
- Security, approval, always-forbidden paths, action-dir isolation, and default
  denial behavior are unchanged and covered by host integration tests.
