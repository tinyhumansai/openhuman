# tinyflows capability seam

Wires the vendored `tinyflows` workflow engine (validate → compile → run over
its own in-crate state-graph runtime, `vendor/tinyflows/`) to real OpenHuman
services. `tinyflows` knows nothing about OpenHuman. Every effect a flow node
can have (calling an LLM, running an agent, making an HTTP request, running
code, calling a tool, reading/writing state, resolving a sub-workflow,
recalling/writing memory) is a trait the engine declares and this module
implements. See
[`gitbooks/developing/architecture/flows-on-tinyagents.md`](../../../../../gitbooks/developing/architecture/flows-on-tinyagents.md)
for the engine's own model (trigger model, run state shape, the two-gate
security model); this README covers only the host seam.

## Layout

- `mod.rs`: export-focused. Re-exports `caps::build_capabilities` and
  `caps::open_flow_checkpointer`, the two entry points `flows::ops*.rs`
  calls to drive a run; re-exports `tinyflows_sqlite::checkpoint` as
  `checkpoint_sqlite` under its historical path.
- `caps/`: every capability adapter except `memory`, plus construction,
  curation, preflight, and invocation logic. `build_capabilities` also fills
  `tasks` with the engine's own `TokioTaskRunner` and deliberately leaves
  `shell` and `approvals` as `None` (no host shell adapter yet; approvals
  reuse the engine's pause/`flows_resume` fallback).
  - `ops.rs`: `build_capabilities` (assembles the `Capabilities` bundle for
    one run), `open_flow_checkpointer` (opens the durable SQLite checkpointer
    at `<workspace_dir>/flows/checkpoints.db`), Composio curation/preflight
    (`is_curated_flow_tool`, `preflight_composio_args`), and the
    `OpenHumanTools` / `PreflightToolInvoker` tool-call adapters.
  - `agent.rs`: `OpenHumanAgentRunner` (`AgentRunner`): resolves an `agent`
    node's trusted `agent_ref` and routes it to a full harness turn
    (`Agent::run_single`, a nested tinyagents graph) when a harness
    `AgentDefinition` exists, or to a persona-shaped `OpenHumanLlm::complete`
    when only a custom registry entry does. Also owns the timeout clamp and
    its scaling against the definition's iteration cap.
  - `llm.rs`: `OpenHumanLlm` (`LlmProvider`) over OpenHuman's inference
    stack; what an `agent` node falls back to without an agent runner, and
    what a raw completion node uses directly.
  - `prompt.rs`: message assembly, the `input_context` carrier and its size
    cap, and tolerant JSON extraction for structured-output nodes.
  - `http.rs`: `OpenHumanHttp` (`HttpClient`); inherits the allowlist and
    DNS-rebind protection of the underlying `HttpRequestTool` and adds
    `http_cred:<name>` credential resolution, injected after the approval
    gate computes its redacted summary so the secret never reaches the UI,
    graph, output, or logs.
  - `code.rs`: `OpenHumanCode` (`CodeRunner`); runs JS/Python through
    `sandbox::execute_in_sandbox` under `CODE_RUN_TIMEOUT_SECS`.
  - `StateStore` is `tinyflows_sqlite::flows::SqliteStateStore` (built in
    `ops.rs`), namespaced per flow (`"flow:<id>"`) so saved flows never
    collide on a state key.
  - `resolver.rs`: `OpenHumanWorkflowResolver` (`WorkflowResolver`);
    resolves a `sub_workflow` node's id to a stored workflow.
  - `tier.rs`: `enforce_node_tier_gate` / `gate_call_for_tier`: the
    autonomy-tier and approval gate every acting node (tool call, HTTP
    request, code run, memory read/write) passes through first.
  - `tools/`: `tool_call` node dispatch, split by slug namespace:
    `native.rs` for the `oh:` prefix (native OpenHuman tools, same registry
    the assistant uses), `composio.rs` for everything else (Composio
    actions; must stay the catch-all backend since Composio slugs carry no
    prefix).
- `memory_adapter.rs`: `OpenHumanMemory`, the `MemoryProvider` adapter for
  the `memory` node. Routes every operation through the same `tier.rs` gate
  pair as the other acting adapters (`CommandClass::Read` for
  `recall`/`search`/`flavour`/`people`, `CommandClass::Write` for
  `remember`/`forget`); `remember`/`forget` hard-refuse any scope other than
  `"flow"`. Scopes are memory v2 tag filters: `user` reads everything,
  `flow` reads the run's own `flow:<id>` items, `flows` reads every flow's
  items (tag `flows`) — the same tags the `flow_memory_*` agent tools use.
  `recall` returns the engine's answer plus citations, `search` raw hits;
  `flavour` and `people` are unsupported in v2 (unknown slug / empty
  listing).
- `observability.rs`: `tinyflows::observability::RunObserver` impls:
  `FlowRunObserver`, which persists live
  steps via `flows::upsert_flow_run_step` and publishes
  `DomainEvent::FlowRunProgress` twice per non-trigger node so the frontend
  can render a run live.
- `langfuse_export.rs`: after a run settles, exports its durable
  `GraphObservation` slice as one Langfuse trace via `tinyagents`'
  `GraphLangfuseExporter`, tagged with the Langfuse Agent Graph view keys.

## Security model

The contract is spelled out in
[flows-on-tinyagents.md § The security model: two gates](../../../../../gitbooks/developing/architecture/flows-on-tinyagents.md#the-security-model-two-gates);
this module holds the outer gate. `tier.rs` consults the user's autonomy
tier for each acting node's `CommandClass` and forces an `ApprovalGate`
round-trip on `Prompt`, even when the saved flow's own `require_approval` is
false. Composio `tool_call` nodes additionally pass `caps/ops.rs`'s
deny-by-default curation check (`is_curated_flow_tool`), stricter than the
agent loop's because a flow author's slug is free-form and never round-trips
through live tool discovery; `tools/composio.rs` documents why the tier gate
must run before curation. The inner gate (the agent definition's `ToolScope`
and sandbox) applies to the nested harness turn `agent.rs` starts, with no
new origin wrapper.

## Used by

`crates/openhuman-core/src/flows/ops*.rs` calls `build_capabilities` and
`open_flow_checkpointer` to run or resume a flow, and
`langfuse_export::export_flow_run_trace` once a run settles.

## Tests

`tinyflows_tests.rs` (capability-seam smoke tests against the real engine;
note the real `HttpRequestTool` blocks loopback, so HTTP coverage asserts the
SSRF/allowlist rejections rather than a mock round-trip),
`memory_adapter_tests.rs` (the `memory` node adapter against an in-memory
reference engine), plus a `<module>_tests.rs` file beside most modules
above.
