# Agent

Multi-agent orchestration domain. Owns the LLM tool-calling loop, sub-agent dispatch, conversation transcripts, the trigger-triage pipeline that classifies incoming external events, and the bundled prompt assets in `agent/prompts/`. Does NOT own provider HTTP transport (`providers/`), tool implementations (`tools/`), prompt section assembly (lives in `context/` — which re-exports from `agent::prompts` via `context::prompt`), or memory storage (`memory/`).

## Public surface

- `pub struct Agent` / `pub struct AgentBuilder` — `harness/session/types.rs` — top-level conversation runtime; entry point for any chat turn.
- `pub mod harness::session::{builder, runtime, turn}` — `harness/session/mod.rs:23-27` — turn lifecycle, fluent builder, `run_single` / `run_interactive`.
- `pub fn run_subagent` / `pub struct SubagentRunOptions` / `pub enum SubagentRunError` — `harness/subagent_runner/` — execute a hierarchical sub-agent from a parent tool loop.
- `pub struct AgentDefinition` / `pub struct AgentDefinitionRegistry` / `pub enum SandboxMode` / `pub enum ToolScope` — `harness/definition.rs` — sub-agent archetypes loaded from built-ins + workspace TOML.
- `pub mod harness::fork_context` — `harness/fork_context.rs` — task-local parent context for KV-cache reuse.
- `pub mod harness::interrupt` (`check_interrupt`, `InterruptFence`, `InterruptedError`) — `harness/interrupt.rs` — graceful cancellation primitives.
- `pub trait ToolDispatcher` / `pub struct ParsedToolCall` / `pub struct ToolExecutionResult` — `dispatcher.rs:14-50` — pluggable tool-call format (XML / JSON / P-Format).
- `pub mod triage` (`run_triage`, `apply_decision`, `TriggerEnvelope`, `TriageDecision`, `TriageAction`) — `triage/mod.rs:34-45` — classify external triggers, escalate to sub-agents.
- `pub mod prompts::SystemPromptBuilder` — `prompts/` — system-prompt section composer.
- `pub mod agents` — `agents/mod.rs` — built-in archetypes (orchestrator, planner, researcher, code_executor, summarizer, archivist, trigger_triage, trigger_reactor, etc.).
- RPC `agent.chat`, `agent.chat_simple`, `agent.server_status`, `agent.list_definitions`, `agent.get_definition`, `agent.reload_definitions`, `agent.triage_evaluate` — `schemas.rs:17-158`.

## Calls into

- `src/openhuman/providers/` — `ChatMessage`, `ChatResponse` send/receive against LLMs.
- `src/openhuman/tools/` — `Tool` / `ToolSpec` execution surface invoked from the tool loop.
- `src/openhuman/memory/` — episodic indexing + memory-loader context injection.
- `src/openhuman/context/` — prompt sections, tool-call format selection.
- `src/openhuman/local_ai/` — `agent_chat` / `agent_chat_simple` execution backend.
- `src/openhuman/config/` — runtime config load via `config::rpc::load_config_with_timeout`.
- `src/core/event_bus/` — emits `DomainEvent::Agent(*)` and `Trigger*` events; subscribers in `agent/bus.rs`.

## Called by

- `src/openhuman/channels/runtime/dispatch.rs` and `channels/providers/web.rs` — drive chat turns from inbound channel messages.
- `src/openhuman/cron/scheduler.rs` — fire scheduled triggers through `triage::run_triage` + `apply_decision`.
- `src/openhuman/webhooks/ops.rs` — webhook ingestion routes through triage.
- `src/openhuman/composio/bus.rs` — Composio trigger envelopes go through `agent::triage`.
- `src/openhuman/notifications/rpc.rs` — surfaces agent runs to the UI.
- `src/openhuman/learning/{reflection,tool_tracker,user_profile}.rs` — read transcripts + tool outcomes.
- `src/openhuman/tools/impl/agent/{dispatch,spawn_subagent}.rs` — `spawn_subagent` tool delegates here.
- `src/core/all.rs` — controller registry wires `all_agent_registered_controllers`.

## Tests

- Unit: `mod.rs` `#[cfg(test)] mod tests;`, `tests.rs`, `multimodal_tests.rs`, `dispatcher_tests.rs`, plus `*_tests.rs` files under `harness/`, `harness/session/`, `triage/`.
- Integration: `tests/agent_builder_public.rs`, `tests/agent_harness_public.rs`, `tests/agent_memory_loader_public.rs`, `tests/agent_multimodal_public.rs`.
- Schema regression: `schemas.rs:393-410` (`controller_schema_inventory_is_stable`).
