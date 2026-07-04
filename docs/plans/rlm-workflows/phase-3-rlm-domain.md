# Phase 3 — The `rlm` domain (`src/openhuman/rlm/`)

New domain following the canonical module shape. Cargo change: root
`Cargo.toml` gains `features = ["sqlite", "repl"]` on the tinyagents dep.

```
src/openhuman/rlm/
├── mod.rs        # exports only + controller schema pair (none in v1)
├── types.rs      # RlmSessionId, RlmRunSummary, RlmLimitsOverride, serde types
├── policy.rs     # autonomy tier + tool_timeout → tinyagents ReplPolicy
├── bridge.rs     # capability bridge: openhuman tools/model/subagents →
│                 # tinyagents CapabilityRegistry<()>
├── sessions.rs   # session manager: persistent ReplSession per session_id
├── ops.rs        # eval_cell orchestration: spawn_blocking, cancel, events
├── tools.rs      # RlmTool (Phase 4)
└── README.md     # module design doc
```

## 3.1 `policy.rs` — mapping openhuman config to `ReplPolicy`

- Base on `ReplPolicy::default()` (already conservative).
- `timeout` = min(caller `timeout_secs` clamped by
  `tool_timeout::explicit_call_timeout_secs`, global cap). Never unbounded.
- `max_agent_calls` / `max_depth` respect `MAX_SPAWN_DEPTH` and the
  autonomy tier: `readonly` tier ⇒ RLM tool not registered at all (see
  Phase 4); `supervised` ⇒ defaults; `full` ⇒ caller may raise limits up to
  a hard ceiling (2× defaults) via the tool's `limits` arg.
- `max_concurrency` capped at 8.
- Log the resolved policy at `debug` with the `[rlm]` prefix.

## 3.2 `bridge.rs` — the capability registry

Builds a `tinyagents::registry::CapabilityRegistry<()>` for a session:

- **Tools**: take the turn's `Vec<Arc<dyn openhuman Tool>>` (the same list
  the harness registered, minus exclusions), wrap each in the existing
  `crate::openhuman::tinyagents::tools::ToolAdapter`, and
  `registry.replace_tool(name, adapter)`. **Exclusions** (recursion +
  duplication guards): `rlm` itself, `spawn_subagent`/`spawn_parallel_agents`
  (use `agent_query` instead), `run_workflow`/`await_workflow`. Because
  `ToolAdapter` carries each tool's own security/approval behavior, scripts
  get exactly the same gates as direct tool calls.
- **Model**: register the turn's `ProviderModel` (and workload routes via
  `routes::build_route_models`) under the same names the harness uses, so
  `model_query(#{model: "chat-v1", ...})` hits the real provider with usage
  accounting intact.
- **Agents**: for each `AgentDefinition` in the parent's
  `allowed_subagent_ids`, register a `SubagentCapability` — a small adapter
  implementing `tinyagents::graph::subagent_node::HarnessAgent` whose
  `run(input, events)` calls
  `agent::harness::subagent_runner::run_subagent(definition, prompt,
  options)` with the parent's workspace descriptor, model override, and
  progress sink threaded through. Depth accounting flows through the
  session's `RunConfig.depth` so `SubAgentDepth` fails closed.

## 3.3 `sessions.rs` — session manager

- `RlmSessionManager` (global `OnceLock`, like `ApprovalGate`): map of
  `session_id -> Mutex<ReplSessionEntry>` where the entry owns the
  `ReplSession<(), ()>`, its `ReplCancelFlag`, creation time, and cell count.
- `get_or_create(session_id, registry, policy)`; sessions are keyed
  per-thread/turn-scope (`<thread_id>:<session_id>`) so parallel chats don't
  share namespaces.
- Eviction: idle TTL (30 min) + LRU cap (16 sessions), enforced on access;
  explicit `close(session_id)`.
- The `ReplSession` is `Send` (rhai `sync` feature) but `eval_cell` takes
  `&mut self` — one cell at a time per session, serialized by the entry
  mutex; a second concurrent call on the same session returns a typed
  "session busy" error rather than queueing forever.

## 3.4 `ops.rs` — evaluating a cell

`pub async fn eval_rlm_cell(req: RlmEvalRequest) -> anyhow::Result<RlmEvalResponse>`

1. Resolve/verify session; register cancel token with
   `workflows::run_log::register_run_cancel`-style bookkeeping tied to the
   turn's cancellation context (`tinyagents/run_cancellation_context.rs`),
   linking user cancel → `ReplCancelFlag::cancel()`.
2. Subscribe an `EventListener` on the session's `EventSink` that forwards
   REPL call events to the turn's `AgentProgress` sink
   (`ToolCallStarted/Completed`-shaped) and publishes coarse
   `DomainEvent`s (start/finish) on the global bus.
3. `tokio::task::spawn_blocking(move || session.eval_cell(&script))` with an
   outer `tokio::time::timeout` at policy timeout + grace (5 s) as a
   belt-and-braces bound (the inner deadline should always fire first).
4. Map `ReplResult` → `RlmEvalResponse { stdout, value, variables_changed,
   calls (summarized), final_answer, elapsed_ms, session_id, cells_used,
   limits_remaining }`, truncating per `max_result_size_chars`.
5. Map every `TinyAgentsError` variant to a **model-consumable** error
   string (Phase 5 details) — errors return as tool results, never panics.

## 3.5 Wiring

- `src/openhuman/mod.rs` (domain list): add `pub mod rlm;`.
- Debug logging throughout with `[rlm]` prefix, correlation fields
  `session_id`, `cell_index`, `thread_id`.
