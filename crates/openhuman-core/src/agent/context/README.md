# context

Global context management for agent sessions: system-prompt assembly, per-session context bookkeeping (utilisation stats, budget configuration), and the shared microcompact constants. Agents hold one `ContextManager` per session. This is a **pure logic / state-tracking domain**: no RPC controllers, no agent tools, no event-bus subscribers, no persisted store.

> **Status (#4249): live history reduction/summarization moved out of this directory.** The in-turn compaction that used to live here (`ContextManager::reduce_before_call`, the `Summarizer` trait, `ProviderSummarizer`, `SegmentRecapSummarizer`, `microcompact.rs`, `pipeline.rs`, `guard.rs`) is gone: see [History](#history-4249). Folding an over-budget transcript into a summary now runs as the vendored `tinyagents_harness::middleware::ContextCompressionMiddleware` (with the seam-owned `ImageAwareMessageTrimMiddleware` as the deterministic backstop) inside `run_turn_via_tinyagents_shared` (`agent/tinyagents/turn_runner.rs`), backed by `agent/tinyagents/summarize.rs::ModelSummarizer`. Tool-result body clearing runs in `tinyagents_harness::middleware::MicrocompactMiddleware`. `stats.rs` keeps the data model behind `ContextManager::stats()`.

## Responsibilities

- Maintain prompt-adjacent session state; system-prompt assembly lives in `agent::prompts`, while `channels_prompt.rs` provides the channel-specific builder.
- Track context-window utilisation per session (`ContextStatsState`).
- Expose the configured per-tool-result byte budget and the compaction/microcompact/autocompact toggles the tinyagents turn reads; enforcement lives in `agent/tinyagents` (`ToolOutputMiddleware`, `MicrocompactMiddleware`, `ContextCompressionMiddleware`) and in `agent/harness/tool_result_artifacts`.
- Expose the microcompact placeholder/keep-recent constants consumed by the microcompact middleware.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/agent/context/mod.rs` | Module docstring, `pub mod` decls, `pub use` re-exports, and the three shared constants (`DEFAULT_TOOL_RESULT_BUDGET_BYTES`, `CLEARED_PLACEHOLDER`, `DEFAULT_KEEP_RECENT_TOOL_RESULTS`). No other logic. |
| `crates/openhuman-core/src/agent/context/manager.rs` | `ContextManager`: per-session handle agents hold. Owns the default prompt builder, `ContextStatsState`, and the `ContextConfig`-derived knobs. Surfaces `build_system_prompt` / `build_system_prompt_tiered`, `stats()`, the getters the tinyagents turn reads (`tool_result_budget_bytes`, `compaction_enabled`, `autocompact_enabled`, `microcompact_keep_recent`, `autocompact_enabled`, `compaction()` (trigger override and strategy), `prefer_markdown_tool_output`), and the session-memory trigger/mark methods. |
| `crates/openhuman-core/src/agent/context/stats.rs` | `ContextStatsState`: last provider usage, context-window utilisation, and the shared `SessionMemoryHandle` (`Arc<Mutex<SessionMemoryState>>`). Pure state; issues no LLM calls and does not mutate history. |
| `crates/openhuman-core/src/agent/context/session_memory.rs` | `SessionMemoryState` / `SessionMemoryConfig`: threshold-gated `should_extract` decision (token growth, tool calls, and turns must all cross) and extraction bookkeeping. Holds `ARCHIVIST_EXTRACTION_PROMPT`. State-tracking only; does not spawn the archivist. |
| `crates/openhuman-core/src/agent/context/channels_prompt.rs` | Bespoke free functions `build_system_prompt(...)` / `build_system_prompt_with_identity(...)` (plus `PromptIdentityOverride`, `ProjectContextPlacement`, `render_project_context`) for channel runtimes (Discord/Slack/Telegram/…). Byte-stable for prefix-cache hits; injects workspace bootstrap files (`SOUL.md`, `IDENTITY.md`, optional `PROFILE.md`/`MEMORY.md`, with profile overrides for `SOUL.md`/`MEMORY.md`), tools, safety, skills, runtime, and channel-capabilities sections. |
| `crates/openhuman-core/src/agent/context/manager_tests.rs`, `channels_prompt_tests.rs`, `session_memory_tests.rs` | Sibling test files wired via `#[cfg(test)] #[path = "..."] mod tests;` from their source file (the layout check rejects inline test modules). |

## History (#4249)

Live history reduction/summarization moved out of this directory into the tinyagents turn path:

- `pipeline.rs`: removed; live context reduction moved to tinyagents middleware, stats state lives in `stats.rs`.
- `guard.rs`: removed; the live 0.90 compression threshold is `SUMMARIZE_THRESHOLD_FRACTION` in `agent/tinyagents/summarize.rs`.
- `microcompact.rs`: removed; live tool-result body clearing is owned by `tinyagents_harness::middleware::MicrocompactMiddleware`, only the shared constants remain in `mod.rs`.
- `tool_result_budget.rs`: removed; UTF-8-safe per-result truncation moved next to action-workspace artifact preview/fallback handling in `agent/harness/tool_result_artifacts`.
- `summarizer.rs`: removed; live summarization moved to `agent/tinyagents/summarize.rs` (`ModelSummarizer`, which also carries the summarizer system prompt).
- `segment_recap_summarizer.rs`: removed; the recap-backed compaction wrapper is gone.
- `summarizer_tests.rs` / `segment_recap_summarizer_tests.rs`: removed along with the files above.

## Public surface

From `mod.rs` re-exports:

- Manager: `ContextManager`, `ContextStats`.
- Microcompact config: `CLEARED_PLACEHOLDER`, `DEFAULT_KEEP_RECENT_TOOL_RESULTS`.
- Prompt: `SystemPromptBuilder`, `PromptSection`, `PromptContext`, `PromptTool`, `ArchetypePromptSection`, `DateTimeSection`, `IdentitySection`, `LearnedContextData`, `RuntimeSection`, `SafetySection`, `ToolsSection`, and `WorkspaceSection` are imported directly from `agent::prompts`.
- Tool-result budget: `DEFAULT_TOOL_RESULT_BUDGET_BYTES` config default only; live truncation is owned by `ToolOutputMiddleware` / `tool_result_artifacts`.

`ContextStatsState` is `pub(crate)` and not re-exported.

## RPC / controllers

None. No `schemas.rs`, no `all_controller_schemas`, no `handle_*`. The `ContextStats` doc comment mentions an "optional `context.get_stats` RPC", but no such schema or handler exists anywhere in the crate.

## Agent tools

None. No `tools.rs`.

## Events

None. No `bus.rs`; no `DomainEvent`s published or subscribed.

## Persistence

No `store.rs`. State is per-session and in-memory:

- `ContextStatsState` holds last token counts and the context window.

## Dependencies

- `crate::config`: `ContextConfig` (`config/schema/context.rs`): `enabled`, `microcompact_enabled` / `microcompact_keep_recent`, `autocompact_enabled`, `compaction_trigger_tokens` (absolute summarization trigger for benchmarks; env `OPENHUMAN_COMPACTION_TRIGGER_TOKENS`), `compaction_strategy` (env `OPENHUMAN_COMPACTION_STRATEGY`), `compaction_enabled`, `tool_result_budget_bytes`, `prefer_markdown_tool_output`, `summarizer_model`, and the embedded `session_memory: SessionMemoryConfig`.
- `crate::inference::provider::UsageInfo`: fed into `record_usage`.
- `crate::agent::prompts`: owns prompt rendering; `channels_prompt.rs` uses `inject_inline_content`.
- `crate::skills::Workflow`: `channels_prompt.rs` renders the available-skills section from a `&[Workflow]`.

## Used by

- `agent::harness`: the primary consumer: `session/builder/builder_build.rs` constructs the `ContextManager`; `session/turn/core/harness_turn.rs` reads the budget/compaction getters into the tinyagents turn config; `session/turn/session_io/{transcript_persist,background_tasks}.rs` read `stats()`; `fork_context.rs` and `../subagent_host/` consume prompt types.
- `agent::tinyagents`: `harness_assembly.rs` reads `CLEARED_PLACEHOLDER` when configuring `MicrocompactMiddleware`; `payload_summarizer.rs` and `host/context_composer.rs` build `PromptContext`s through `agent::prompts`.
- **`agent::registry::agents/*/prompt.rs`**, `flows/agents/*/prompt.rs`, `skills/*/agent/*/prompt.rs`: archetype prompt modules pull prompt sections/builder through `agent::prompts`.
- `channels`: `channels/system_prompt.rs` (`ChannelSystemPrompt`, used by `channels/runtime/startup/start_channels.rs`) calls `channels_prompt::build_system_prompt_with_identity`; `channels/mod.rs` re-exports `build_system_prompt`; the `channels/tests/*` prompt/identity tests call it directly.
- `agent::triage`, `agent::debug`, `agent::orchestration::tools` (spawn_subagent and friends), `tools::orchestrator_tools`, `integrations::composio`: consume prompt types (`PromptContext`, `PromptSection`, `ToolCallFormat`, `ConnectedIntegration`, …) via `agent::prompts`.

## Notes / gotchas

- **The context stats state issues no LLM calls and does not mutate history.** Live reduction is owned by the tinyagents middleware stack.
- **Tool-result budgeting is not a context pipeline stage.** The tinyagents path applies per-result budgets in `ToolOutputMiddleware` (`agent/tinyagents/middleware/tool_output.rs`), and artifact-preview fallback truncation lives in `agent/harness/tool_result_artifacts`.
- **`autocompact_enabled()` is `config.enabled && config.autocompact_enabled`**, and `microcompact_keep_recent()` is `0` when `microcompact_enabled` is off: the manager folds the config gates so the turn reads one value per knob.
- Prompt rendering does not belong in this module; add it to `agent::prompts`.
- **`channels_prompt::build_system_prompt` deliberately bypasses `SystemPromptBuilder`** to keep production channel prompt bytes stable for prefix-cache hits; it is a standalone free function despite living under `context/`.
