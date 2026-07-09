# Phase 1 — Research findings

## 1. What TinyAgents' `repl` feature provides

Feature gate: `repl = ["dep:rhai"]` with `rhai = { version = "1", features =
["sync"] }` (`vendor/tinyagents/Cargo.toml`). The `sync` feature makes engine
and values `Send + Sync` so a session can live inside an async task.

There are **two** `ReplSession` types; we use the scripting one:

- `tinyagents::repl::ReplSession` — line-oriented command *skeleton*
  (`load/compile/run/call` return `ReplOutcome::Planned`, never executed).
  Not our target.
- `tinyagents::repl::session::ReplSession` (crate-root re-export
  `tinyagents::ReplSession` under the feature) — the **Rhai scripting
  session**. This is the RLM/CodeAct runtime.

### Session lifecycle

- `ReplSession::from_parts(capabilities, policy, run_context)`; builders
  `with_capabilities` / `with_policy` / `with_state` rebuild the engine.
- `eval_cell(&mut self, script) -> Result<ReplResult>` — one cell per call;
  top-level `let` bindings persist across cells (persistent `rhai::Scope`).
- `ReplResult { stdout, value, variables_changed, calls, final_answer,
  elapsed }` — typed, serializable (`ReplValue` = Unit/Bool/Int/Float/
  String/Array/Map).
- Reserved names (`context`, `state`, `messages`, `history`, `run` + 16
  capability functions) are restored after every cell — scripts can shadow
  but never permanently replace capabilities.

### Script-visible built-ins (`src/repl/session/builtins/`)

| Built-in | Lowers to |
| -------- | --------- |
| `model_query(#{model, system?, prompt?, structured?})` | `registry.model(name).invoke(...)` |
| `tool_call(#{tool, arguments?})` | `registry.tool(name).call(...)` |
| `agent_query(#{agent, prompt})` | `registry.agent(name).run(SubAgentInput, events)` |
| `model_query_batched / tool_call_batched / agent_query_batched([...])` | bounded concurrency via `stream::iter(..).buffered(max_concurrency)` |
| `graph_run / graph_define / graph_validate / graph_compile / graph_diff / graph_register` | `.rag` compiler + resolver + review gate (graph_run returns a *reference*, does not execute) |
| `emit(name, #{..})`, `answer(content)`, `show_vars()`, `print/debug` | recorded into `ReplResult` |

Bridge mechanism: an `Arc<HostContext<State>>` (registry, state, policy,
`EventSink`, shared `CellBuffers`) is cloned into every registered closure;
results flow back through `Arc<Mutex<..>>` buffers.

### Fail-closed limits (`ReplPolicy`, defaults)

`max_operations` 1M · `max_iterations` 16 · `max_script_bytes` 64 KiB ·
`max_output_bytes` 256 KiB · `max_model_calls` 64 · `max_agent_calls` 32 ·
`max_tool_calls` 128 · `max_graph_calls` 32 · `max_depth` 8 · `timeout`
30 s · `max_concurrency` 4 · `generated_graphs_require_review` true.

Timeout is enforced at **two points**: the engine `on_progress` hook (pure
script loops) and `bridge_block_on` (a timer-thread race that drops the
in-flight capability future — cancel-safe reqwest). Precise
`TinyAgentsError`s (Timeout, LimitExceeded, ModelNotFound, SubAgentDepth, …)
are stashed via `CellBuffers::set_host_error` and surfaced verbatim.

### Async story

The engine is synchronous; capability calls run through a **blocking
bridge** (`futures::executor::block_on`). `eval_cell` must therefore run on
`tokio::task::spawn_blocking` (or a dedicated thread) — calling it on an
async worker deadlocks a current-thread runtime.

### Gaps a host must fill (drives Phase 2)

1. **No external cancellation** — only per-cell wall-clock timeout; no
   abort handle to stop a running cell on demand.
2. **No live progress** — `stdout`/`calls`/`emit` are only readable after
   the cell returns; nothing streams on the `EventSink` mid-cell.
3. **No CodeAct driver loop** — the host owns the "model writes cell →
   eval → feed result back" loop (in our design, the orchestrator's normal
   tool-call loop *is* that loop; each `rlm` tool call is one cell).
4. `graph_run` doesn't execute compiled graphs (returns a reference map) —
   out of scope for v1; we expose model/tool/agent capabilities only.
5. Sync `eval_cell` + internal `block_on` — handled host-side with
   `spawn_blocking`, documented in tinyagents as part of Phase 2.

## 2. What OpenHuman provides (integration points)

- **Dependency**: `tinyagents = { version = "1.5.0", features = ["sqlite"] }`
  patched to `path = "vendor/tinyagents"` (git submodule,
  `tinyhumansai/tinyagents`). We add the `"repl"` feature.
- **Tool trait** (`src/openhuman/tools/traits.rs:255`): `name` /
  `description` / `parameters_schema` / `async execute` (+
  `execute_with_context`, `permission_level_with_args`, `external_effect`,
  `timeout_policy`, `display_label/detail`). Registered by adding one
  `Box::new(...)` line in `all_tools_with_runtime`
  (`src/openhuman/tools/ops.rs`).
- **Tool→tinyagents bridge already exists**: `ToolAdapter`
  (`src/openhuman/tinyagents/tools.rs:78`) wraps `Arc<dyn openhuman Tool>`
  and implements `tinyagents::Tool<()>` — we reuse it to project openhuman
  tools into the REPL's `CapabilityRegistry`.
- **Model bridge already exists**: `ProviderModel`
  (`src/openhuman/tinyagents/model.rs`) implements the tinyagents model
  trait over openhuman's `Provider`; `assemble_turn_harness`
  (`src/openhuman/tinyagents/mod.rs:1122`) already builds a
  `CapabilityRegistry<()>` per turn with models registered.
- **Subagents**: `run_subagent(definition, prompt, options)`
  (`src/openhuman/agent/harness/subagent_runner/`) + parent allowlist
  (`allowed_subagent_ids`) + `MAX_SPAWN_DEPTH`. We wrap this in a
  `HarnessAgent` impl so `agent_query("researcher", ...)` spawns real
  openhuman subagents.
- **Timeout/cancel**: `tool_timeout` domain clamps 1–3600 s;
  `workflows::run_log::register_run_cancel(run_id) -> CancellationToken`.
- **Approval/security**: `external_effect_with_args == true` routes through
  the `ApprovalGate` middleware; per-tool security is enforced inside each
  tool via `Arc<SecurityPolicy>` — bridged tools keep their own checks, so
  the REPL inherits them for free.
- **Progress**: per-turn `AgentProgress` mpsc sink (streams to UI + run
  logs) and the global `DomainEvent` bus (`ToolExecutionStarted/Completed`,
  `Workflow*` events).
- **Prompt surfacing**: tool `description()` + `parameters_schema()` ride in
  the native tool-call API request; the orchestrator's narrative guide is
  `src/openhuman/agent_registry/agents/orchestrator/prompt.md` + `agent.toml`.
- **No existing rhai/RLM surface** in `src/` — this is net-new, but it sits
  beside `workflows/`, `flows/`, `tinyflows/`, `agent_orchestration/`.

## 3. Design decision: one cell per tool call

The orchestrator's existing turn loop already is a CodeAct loop. So the
`rlm` tool maps **one tool call → one `eval_cell`**, with an optional
persistent `session_id` so a later call continues the same namespace
(`let findings = ...` in cell 1, referenced in cell 2). This avoids building
a bespoke driver loop, keeps every cell inside the normal approval/permission
middleware, and gives the model natural iteration with feedback.
