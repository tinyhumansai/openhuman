# context

Global context management for agent sessions: the home for system-prompt assembly, per-session context bookkeeping (utilisation stats, budget configuration, session-memory triggers), and prompt-cache diagnostics. Agents hold one `ContextManager` per session. This is a **pure logic / state-tracking domain** — no RPC controllers, no agent tools, no event-bus subscribers, no persisted store.

> **Status (#4249): live history reduction/summarization moved to the tinyagents graph.** The in-turn compaction that used to live here — `ContextManager::reduce_before_call`, the `Summarizer` trait, `ProviderSummarizer`, `SegmentRecapSummarizer`, `context/microcompact.rs`, `context/pipeline.rs`, and `context/guard.rs` — has been **removed**. Folding an over-budget transcript into a summary now runs as `ContextCompressionMiddleware` (+ `MessageTrimMiddleware` backstop) inside `run_turn_via_tinyagents_shared`, backed by `tinyagents::summarize::ProviderModelSummarizer`. Tool-result body clearing now runs in TinyAgents `MicrocompactMiddleware`; `context/stats.rs` keeps the data model behind `ContextManager::stats()` (the utilisation footer) and session-memory bookkeeping.

## Responsibilities

- Assemble opening system prompts (delegates to `agent::prompts` via `prompt.rs`; provides a separate bespoke builder for channel runtimes).
- Track context-window utilisation per session (`ContextStatsState`).
- Expose the configured per-tool-result byte budget; enforcement now lives in TinyAgents tool-output middleware and action-workspace artifact previews.
- Expose microcompact configuration/placeholder constants consumed by TinyAgents middleware.
- Maintain session-memory and utilisation bookkeeping (`stats.rs`).
- Track session-memory extraction thresholds (token growth, tool calls, turns) and report when a background archivist extraction should fire — without spawning it itself (`session_memory`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/context/mod.rs` | Module docstring + `pub mod` decls + `pub use` re-exports. Export-focused, no logic. |
| `src/openhuman/context/manager.rs` | `ContextManager` — per-session handle agents hold. Owns the default prompt builder, stats/session-memory state, and budget/markdown config. Surfaces `build_system_prompt`, `stats()`, tool-result budget settings, and session-memory triggers. (The former `reduce_before_call`/summarizer dispatch was removed in #4249.) |
| `src/openhuman/context/stats.rs` | `ContextStatsState` — provider usage, context-window utilisation, and shared `SessionMemoryHandle` bookkeeping. Pure state; issues no LLM calls and does not mutate history. |
| `src/openhuman/context/pipeline.rs` | **Removed (#4249).** Live context reduction moved to TinyAgents middleware; stats/session-memory state lives in `context/stats.rs`. |
| `src/openhuman/context/guard.rs` | **Removed (#4249).** The live 0.90 compression threshold is mirrored by `tinyagents::summarize::SUMMARIZE_THRESHOLD_FRACTION`. |
| `src/openhuman/context/microcompact.rs` | **Removed (#4249).** Live tool-result body clearing is owned by TinyAgents `MicrocompactMiddleware`; only shared constants remain in `context/mod.rs`. |
| `src/openhuman/context/tool_result_budget.rs` | **Removed (#4249).** UTF-8-safe per-result truncation moved next to action-workspace artifact preview/fallback handling in `agent/harness/tool_result_artifacts`. |
| `src/openhuman/context/summarizer.rs` | **Removed (#4249).** Live summarization moved to `tinyagents::summarize` (`ProviderModelSummarizer`); the summarizer system prompt was relocated there. |
| `src/openhuman/context/segment_recap_summarizer.rs` | **Removed (#4249).** The archivist-recap-backed compaction wrapper is gone; the archivist still produces durable segment recaps on its own post-turn path. |
| `src/openhuman/context/session_memory.rs` | `SessionMemoryState` / `SessionMemoryConfig` — threshold-gated `should_extract` decision (token growth + tool calls + turns must all cross) and extraction bookkeeping. Holds `ARCHIVIST_EXTRACTION_PROMPT`. State-tracking only; does not spawn the archivist. |
| `src/openhuman/context/prompt.rs` | Compat shim — `pub use crate::openhuman::agent::prompts::*`. Prompt rendering moved to `agent::prompts`; this keeps `context::prompt::...` as a stable import path. |
| `src/openhuman/context/channels_prompt.rs` | Bespoke free-function `build_system_prompt(...)` for channel runtimes (Discord/Slack/Telegram/…). Byte-stable for prefix-cache hits; injects OpenClaw bootstrap files (`SOUL.md`, `IDENTITY.md`, optional `PROFILE.md`/`MEMORY.md`), tools, safety, skills, runtime, and channel-capabilities sections. |
| `src/openhuman/context/manager_tests.rs` | Sibling test suite wired via `#[cfg(test)] #[path = ...] mod tests`. Other files use inline `#[cfg(test)] mod tests`. (`summarizer_tests.rs` / `segment_recap_summarizer_tests.rs` removed in #4249.) |

## Public surface

From `mod.rs` re-exports:

- **Manager**: `ContextManager`, `ContextStats`.
- **Microcompact config**: `CLEARED_PLACEHOLDER`, `DEFAULT_KEEP_RECENT_TOOL_RESULTS`.
- **Stats**: `ContextStats` via `ContextManager::stats()`; internal
  bookkeeping stays in `ContextStatsState` / `SessionMemoryHandle`.
- **Prompt** (re-exported from `agent::prompts`): `SystemPromptBuilder`, `PromptSection`, `PromptContext`, `PromptTool`, `ArchetypePromptSection`, `DateTimeSection`, `IdentitySection`, `LearnedContextData`, `RuntimeSection`, `SafetySection`, `ToolsSection`, `WorkspaceSection`.
- **Session memory**: `SessionMemoryConfig`, `SessionMemoryState`, `ARCHIVIST_EXTRACTION_PROMPT`, `DEFAULT_MIN_TOKEN_GROWTH`, `DEFAULT_MIN_TOOL_CALLS`, `DEFAULT_MIN_TURNS_BETWEEN`.
- **Tool-result budget**: `DEFAULT_TOOL_RESULT_BUDGET_BYTES` config default only; live truncation logic is owned by TinyAgents tool-output middleware / tool-result artifacts.

## RPC / controllers

None. No `schemas.rs`, no `all_controller_schemas`, no `handle_*`. `ContextStats` doc comments reference an optional `context.get_stats` / `context.get_stats` RPC, but the schema/handler is not defined in this module.

## Agent tools

None. No `tools.rs`. (Session-memory extraction uses the `update_memory_md` / `memory_recall` / `memory_search` tools, but those are owned elsewhere; this module only references them in prompt text.)

## Events

None. No `bus.rs`; no `DomainEvent`s published or subscribed.

## Persistence

No `store.rs`. State is per-session and in-memory:

- `ContextStatsState` holds last token counts, context window, and the shared
  `SessionMemoryHandle`.
- `SessionMemoryState` (behind a shared `Arc<Mutex<…>>` `SessionMemoryHandle`) tracks cumulative tokens / tool calls / turn counters and extraction-in-progress flag; resets naturally when a session ends.

The durable long-term substrate session-memory targets is the workspace `MEMORY.md` file, but that file is written by the spawned archivist sub-agent (owned by the agent harness), not by this module.

## Dependencies

- `crate::openhuman::config` — reads `ContextConfig` (`config/schema/context.rs`): enabled flag, microcompact/autocompact toggles, `summarizer_model`, `tool_result_budget_bytes`, `prefer_markdown_tool_output`, and embedded `SessionMemoryConfig` thresholds.
- `crate::openhuman::inference::provider` — core types including `UsageInfo`.
- `crate::openhuman::agent::prompts` — `prompt.rs` re-exports the entire prompt-section/builder surface from here (prompt logic lives next to the agents that consume it).
- `crate::openhuman::skills::Skill` — `channels_prompt.rs` renders the available-skills section.
## Used by

- **`agent::harness`** — the primary consumer: `session/builder.rs` constructs the `ContextManager`; `session/turn.rs` drives the session-memory counters and spawns the archivist extraction when `should_extract_session_memory` says so; `fork_context.rs`, `subagent_runner/*`, and `tool_filter.rs` consume the prompt builder and stats/budget surface. (History reduction/summarization moved to the tinyagents graph in #4249 — see the status banner above; `reduce_before_call`/`ProviderSummarizer`/`SegmentRecapSummarizer`/`unified_compaction_enabled` are removed.)
- **`agent::agents/*/prompt.rs`** — every archetype prompt module pulls prompt sections/builder through `context::prompt`.
- **`channels`** — `channels/runtime/startup.rs` (and channel prompt/identity tests) call `channels_prompt::build_system_prompt`.
- **`agent::dispatcher`, `agent::triage`, `agent::tools` (spawn_subagent / spawn_parallel / spawn_worker_thread), `learning::prompt_sections`, `memory_tools::prompt`, `tools::orchestrator_tools`, `composio::ops`** — consume the prompt-building surface.
- **`config::schema`** — `context.rs` embeds `SessionMemoryConfig`.

## Notes / gotchas

- **The context stats state issues no LLM calls and does not mutate history.** Live reduction is owned by TinyAgents middleware.
- **Tool-result budgeting is not a context pipeline stage.** The live TinyAgents path applies per-result budgets in `ToolOutputMiddleware`, and artifact-preview fallback truncation lives in `agent/harness/tool_result_artifacts`.
- **Cache contract**: cache-affecting history rewrites live in TinyAgents middleware, where the run can emit the corresponding events.
- **Session memory is separate from compaction**: it does not mutate in-flight history; it gates a *persistent* `MEMORY.md` extraction. All three thresholds (token growth, tool calls, turns) must be crossed and no extraction may be in flight. `mark_extraction_failed` keeps deltas so the next turn retries; `mark_extraction_complete` resets them. The handle is `Arc`-cloned so a detached background task can flip completion state after the synchronous borrow is released.
- **`prompt.rs` is a compat shim** — do not add prompt logic here; it lives in `agent::prompts`.
- **`channels_prompt::build_system_prompt` deliberately bypasses `SystemPromptBuilder`** to keep production channel prompt bytes stable for prefix-cache hits; it is a standalone free function despite living under `context/`.
