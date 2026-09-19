# Agent

Multi-agent orchestration domain. Owns the LLM tool-calling loop, sub-agent dispatch, conversation transcripts, the trigger-triage pipeline that classifies incoming external events, and the bundled prompt assets in `agent/prompts/`. Does NOT own model construction or provider HTTP transport (`crates/openhuman-core/src/inference/provider/`), tool implementations (`tools/`), or memory storage (`memory/`).

## Public surface

- `pub struct Agent` / `pub struct AgentBuilder` / `pub struct TurnOverrides` — `harness/session/types.rs`, re-exported from `harness::session` and `agent` — top-level conversation runtime; entry point for any chat turn. Constructors `Agent::from_config`, `from_config_for_agent`, `from_config_for_agent_with_profile` live in `harness/session/builder/factory.rs`; `run_single` / `run_interactive` in `harness/session/runtime/run_loop.rs`. The `builder/`, `runtime/`, and `turn/` submodules are private.
- `pub fn run_subagent` / `pub struct SubagentRunOptions` / `pub enum SubagentRunError` — `harness/subagent_runner/` — execute a hierarchical sub-agent from a parent tool loop.
- `pub struct AgentDefinition` / `pub struct AgentDefinitionRegistry` / `pub enum SandboxMode` / `pub enum ToolScope` — `harness/definition/` (`agent_definition.rs`, `registry.rs`, `source.rs`, `tier.rs`, `execution_spec.rs`, `prompt_source.rs`, `subagents.rs`) — sub-agent archetypes loaded from built-ins + workspace TOML.
- `pub mod harness::fork_context` — task-local parent context for KV-cache reuse.
- `pub trait ToolDispatcher` / `pub struct ParsedToolCall` / `pub struct ToolExecutionResult` — `dispatcher.rs` — pluggable tool-call format (XML / JSON / P-Format).
- `pub mod triage` (`run_triage`, `apply_decision`, `TriggerEnvelope`, `TriageDecision`, `TriageAction`) — `triage/mod.rs` — classify external triggers, escalate to sub-agents.
- `pub mod prompts::SystemPromptBuilder` — `prompts/` — system-prompt section composer.
- `pub struct ChatMessage` / `pub enum ConversationMessage` / `pub struct ToolResultMessage` — `messages.rs` — transcript wire types; `inference/provider/types.rs::ChatRequest` borrows `&[ChatMessage]` from here.
- `pub fn bus::register_agent_handlers` — `bus.rs` — registers the `agent.run_turn` native request handler (`AgentTurnRequest` → `AgentTurnResponse`) on `BUS.native()`; called from `channels/runtime/startup/start_channels.rs`.
- Built-in archetypes live in `crates/openhuman-core/src/agent/registry/agents/`; this module stays focused on harness/runtime behavior.
- RPC `agent.{chat, chat_simple, server_status, list_definitions, get_definition, reload_definitions, triage_evaluate, graph_topologies, registry_snapshot}` — `schemas.rs`.
- Read-only replay RPC `agent.{runs_active, run_status, run_events}` — `tinyagents/replay/schemas.rs` — pages a run's durable journal/status without holding the run open.

## Submodule map

| Path | Purpose |
| --- | --- |
| `artifacts/` | Agent-generated artifact storage, retrieval, and lifecycle ([README](artifacts/README.md)) |
| `context/` | System-prompt assembly and per-session `ContextManager` bookkeeping (utilisation stats, budget, session-memory triggers) ([README](context/README.md)) |
| `debug/` | Renders the exact system prompt a live session would see for a given agent, via `Agent::from_config_for_agent` |
| `experience/` | Local procedural operating experience capture for self-learning ([README](experience/README.md)) |
| `file_state/` | Process-wide read/write stamps so parallel sub-agents and worker threads detect stale file contents before writing |
| `git_attribution/` (`pub(crate)`) | Temporary `prepare-commit-msg` hook that appends OpenHuman's co-author trailer to agent-made commits |
| `harness/` | `Agent`/`AgentBuilder`, session lifecycle, sub-agent runner, definitions, fork context — the tool-calling loop itself ([README](harness/README.md)) |
| `harness_init/` | One-time first-run provisioning (Python/spaCy/Kompress/Node) before the harness can run ([README](harness_init/README.md)) |
| `learning/` | Reflection, tool-outcome tracking, user-profile inference from transcripts ([README](learning/README.md)) |
| `library/` | Safe, user-facing projection of agent definitions (`AgentDefinitionDisplay`) |
| `orchestration/` | Command center, workflow runs, agent teams, worktrees, subagent control, `spawn_subagent` and its sibling tools ([README](orchestration/README.md)) |
| `plan_review/` | Interactive plan-review gate that parks a live turn on a thread-scoped plan |
| `profiles/` | Persistent agent profiles (name, soul, memory sources, skills, MCP, connectors) ([README](profiles/README.md)) |
| `progress_tracing.rs` + `progress_tracing/` (`pub(crate)`) | Structured OpenTelemetry/Langfuse-style spans off the `progress::AgentProgress` stream ([README](progress_tracing/README.md)) |
| `prompts/` | Prompt types, section builders, `SystemPromptBuilder` ([README](prompts/README.md)) |
| `registry/` | User-facing agent registry: defaults, enablement, custom agents, tool policy; `registry/agents/` holds built-in archetypes ([README](registry/README.md)) |
| `session_db/` | `run_ledger` RPC controllers over the durable run ledger; the store itself lives in `tinyagents::session::run_ledger` |
| `session_import/` | One-time import of legacy OpenHuman session JSONL/Markdown into TinyAgents stores ([README](session_import/README.md)) |
| `task_dispatcher/` | Claims a `TaskBoardCard` via compare-and-set and runs one autonomous agent turn against it; `start_board_poller` drives it in the background ([README](task_dispatcher/README.md)) |
| `tinyagents/` | Integration with the vendored `tinyagents` loop/replay crate: `TurnModelSource`, middleware, journal, `replay/schemas.rs` ([README](tinyagents/README.md)) |
| `tools/` | Agent-loop control tools (`ask_clarification`, `delegate`, `delegate_to_personality`, `plan_exit`, `remember_preference`, `save_preference`, `run_workflow`, `todo`, `update_task`), re-exported through `crate::tools` |
| `triage/` | Classifies external `TriggerEnvelope`s and escalates to sub-agents ([README](triage/README.md)) |

Flat files: `bus.rs` (`agent.run_turn` native request handler), `cost.rs` (`pub(crate)`, per-turn token/cost accounting), `dispatcher.rs` (tool-call format dispatch), `error.rs` (typed retryable/permanent loop errors), `hooks.rs` (post-turn self-learning hooks), `host_runtime.rs` (native shell execution backend), `message_convert.rs` (`pub(crate)`, transcript ↔ TinyAgents `Message` conversion), `messages.rs` (transcript types), `multimodal.rs` (attachment handling), `pformat.rs` (adapter over `tinyagents_harness::tool_calling::pformat`), `platform_shell.rs` (cross-platform shell selection shared with `host_runtime` and `sandbox::ops`), `progress.rs` (`AgentProgress` channel), `progress_sink.rs` (task-local progress sink for in-process embedders), `stop_hooks.rs` (mid-turn policy halts), `task_board.rs` (per-thread task board over `tinyagents_graph::todos`), `task_session.rs` (`pub(crate)`, task-board runs as conversation threads), `tool_policy.rs` (pre-execution tool-call policy hook), `turn_origin.rs` (task-local trust/routing label read by the approval gate), `turn_workspace.rs` (task-local per-turn filesystem root).

## RPC namespaces owned by this tree

`agent`, `agent_registry`, `profiles`, `harness_init`, `session_import`, `plan_review`, `run_ledger` (session_db), `agent_experience` (experience), `ai` (artifacts), `learning`, `agent_team`, `agent_work` (orchestration/command_center), `workflow_run`, `worktree`, `subagent` (orchestration/subagent_control) — all registered under `DomainGroup::Agent` in `core/all.rs`.

`crate::rpc` is `pub use openhuman_rpc as rpc` in `lib.rs`; shared RPC contracts, response decoding, and the HTTP client live in the separate `crates/openhuman-rpc` crate, not under `agent/`.

## Calls into

- `crates/openhuman-core/src/inference/provider/` — `factory::{provider_for_role, create_chat_model_with_model_id}` build the crate-native `ChatModel`s that `tinyagents::TurnModelSource` runs each turn against; `ChatResponse` / `ToolCall` / `UsageInfo` DTOs cross this boundary. There is no `Provider` trait — the harness names crate model types only.
- `crates/openhuman-core/src/tools/` — `Tool` / `ToolSpec` execution surface invoked from the tool loop.
- `crates/openhuman-core/src/memory/` — episodic indexing + memory-loader context injection (`harness/memory_context.rs`).
- `crates/openhuman-core/src/inference/local/` — `agent_chat` / `agent_chat_simple` execution backend.
- `crates/openhuman-core/src/config/` — runtime config load via `config::rpc::load_config_with_timeout` (`config::rpc` is `pub use ops as rpc`).
- `crates/openhuman-core/src/core/bus.rs` (`BUS.publish`/`BUS.subscribe`/`BUS.native()`) and `crates/openhuman-core/src/core/events.rs` (`DomainEvent`) — emits `AgentTurnStarted` / `AgentTurnCompleted` / `AgentError`, `AgentOrchestration*`, and `TriggerEvaluated`; subscribers live in `orchestration/{background_delivery,run_ledger_finalize}.rs` and `learning/`, not in `agent/bus.rs`.

## Called by

- `crates/openhuman-core/src/channels/runtime/dispatch/` (`processor*.rs`, `routing.rs`) — drives chat turns through the `agent.run_turn` native handler; `web_chat/` (`session.rs`, `run_task.rs`) builds `Agent`s directly.
- `crates/openhuman-core/src/cron/scheduler/agent_run.rs::run_agent_job` — builds an `Agent` directly via `Agent::from_config_for_agent[_with_profile]` / `Agent::from_config` and delivers output through `scheduler/delivery.rs::deliver_if_configured`; it does not go through triage.
- `crates/openhuman-core/src/skills/webhooks/{ops,bus}.rs` — webhook ingestion routes through `triage::run_triage` + `apply_decision`.
- `crates/openhuman-core/src/memory/sync/composio/bus*.rs` — Composio trigger envelopes go through `agent::triage`.
- `crates/openhuman-core/src/integrations/task_sources/route.rs` — external task-source events go through the same `TriggerEnvelope` → `run_triage` → `apply_decision` path.
- `crates/openhuman-core/src/desktop/notifications/rpc.rs` — `notification_ingest` kicks off background triage to back-fill the notification score.
- `crates/openhuman-core/src/agent/schemas.rs::handle_triage_evaluate` — `agent.triage_evaluate`, the dry-run triage entry point exposed over RPC.
- `crates/openhuman-core/src/agent/learning/{reflection,tool_tracker,user_profile}.rs` — read transcripts + tool outcomes.
- `crates/openhuman-core/src/agent/orchestration/tools/{dispatch,spawn_subagent}.rs` — `spawn_subagent` tool delegates to `harness::subagent_runner`.
- `crates/openhuman-core/src/core/runtime/services.rs` — starts `agent::task_dispatcher::start_board_poller` and runs `agent::harness_init::run_harness_init` during core startup.
- `crates/openhuman-core/src/core/all.rs` — controller registry wires all `agent`, `agent_registry`, `profiles`, `harness_init`, `plan_review`, `artifacts`, `experience`, `learning`, `session_db`, `session_import`, and `orchestration` controllers under `DomainGroup::Agent`.

## Tests

- Unit: `agent_tests.rs`, `multimodal_tests.rs`, `dispatcher_tests.rs`, plus `*_tests.rs` files colocated with `bus.rs`, `cost.rs`, `error.rs`, `hooks.rs`, `host_runtime.rs`, `message_convert.rs`, `pformat.rs`, `platform_shell.rs`, `progress_sink.rs`, `schemas.rs`, `stop_hooks.rs`, `task_board.rs`, `task_session.rs`, `tool_policy.rs`, `turn_origin.rs`, `turn_workspace.rs`, and under `harness/`, `harness/session/`, `triage/`.
- Integration: `tests/agent_builder_public.rs`, `tests/agent_harness_public.rs`, `tests/agent_harness_e2e.rs`, `tests/agent_multimodal_public.rs`, `tests/agent_turn_overrides_e2e.rs`, `tests/agent_approval_memory_coverage_e2e.rs`.
- Schema regression: `schemas_tests.rs` (`controller_schema_inventory_is_stable`).

## Related docs

- [gitbooks/developing/architecture/agent-harness.md](../../../../gitbooks/developing/architecture/agent-harness.md)
- [gitbooks/developing/agent-observability.md](../../../../gitbooks/developing/agent-observability.md)

### Scoped tool-call budgets for embedders

`stop_hooks::with_tool_call_limit(Some(n), turn)` narrows the real TinyAgents
invocation budget for one awaited turn without changing the agent's persistent
configuration. Zero permits no tool invocations. The adapter applies the limit
to both run policy and run configuration, including parallel calls counted by
TinyAgents. `with_stop_hooks_and_tool_limit` combines it with stop hooks.

Nested scopes take the smaller limit; `None` preserves an enclosing limit.
Exiting or dropping the future restores the caller's scope, and concurrent
turns do not share limits. This bounds calls within each run, not a shared
aggregate across child runs. Task-local values do not automatically propagate
through `tokio::spawn`; callers creating a separate task must scope that turn
explicitly. Without a limit, existing iteration-derived limits are unchanged.
