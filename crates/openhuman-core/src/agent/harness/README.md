# harness

Product shell around the tinyagents tool loop. The model/tool iteration
itself runs in `crate::agent::tinyagents` (via
`run_turn_via_tinyagents_shared`); this module owns everything OpenHuman
layers on top: the `Agent`/`AgentBuilder` session, transcript persistence and
legacy-format migration, sub-agent dispatch, agent archetypes, post-turn
memory/archival hooks, mid-turn message steering, and oversized-tool-result
handling.

Cancellation is the tinyagents steering channel (`SteeringCommand` in
`crate::agent::tinyagents`); there is no in-house interrupt fence or
cancellation token owned here. `run_queue` only queues messages between
iterations.

## Responsibilities

- Own the `Agent` struct and its per-turn lifecycle: prompt/context assembly,
  tool dispatch, transcript persistence, KV-cache prefix stability
  (`session/`).
- Dispatch sub-agents: resolve an `AgentDefinition`, filter tools, build a
  narrow prompt, run the child turn, and return one compact tool result to
  the parent (`subagent_runner/`).
- Define sub-agent archetypes (built-in + workspace TOML) and the task-local
  plumbing that lets a spawned tool see its parent's runtime context
  (`definition*.rs`, `builtin_definitions.rs`, `fork_context.rs`,
  `sandbox_context.rs`, `spawn_depth_context.rs`, `task_recency_context.rs`,
  `OpenHumanRunContext`).
- Run the channel/CLI turn graph (`graph.rs`) and let a built-in agent
  select a bespoke sub-agent turn graph (`agent_graph.rs`).
- Extract lessons and episodic memory after each turn as a `PostTurnHook`
  (`archivist/`).
- Offload oversized worker artifacts to the filesystem, and persist oversized
  tool results as action-workspace artifacts (`artifact_offload/`,
  `tool_result_artifacts/`).
- Queue mid-turn messages into steer/followup/collect lanes without aborting
  the in-flight turn (`run_queue/`).

## Key sub-modules

| Module | Role |
| --- | --- |
| `session/` | `Agent`/`AgentBuilder`/`TurnOverrides` (`types.rs`), the fluent builder + `Agent::from_config` factory (`builder/`), the turn lifecycle (`turn/`: `context.rs`, `core*.rs`, `graph.rs`, `recall_lanes.rs`, `session_io*.rs`, `tools.rs`), host-side transcript persistence + legacy migration (`session_io/`, `migration.rs`, `turn_checkpoint.rs`). Durable transcript codecs and discovery live in `tinyagents_session::transcript`. |
| `subagent_runner/` | `run_subagent`/`SubagentRunOptions`/`SubagentRunError` and the build pipeline around the tinyagents graph: model resolution, tool filtering, sandbox/action-root narrowing, checkpoint/handback, transcript mirroring (`autonomous.rs`, `extract_tool.rs`, `handoff.rs`, `tool_prep.rs`, `ops/{provider,prompt,runner,graph,checkpoint,pause_checkpoint}.rs`). |
| `definition*.rs`, `builtin_definitions.rs`, `definition_loader.rs` | `AgentDefinition`/`AgentDefinitionRegistry`/`SandboxMode`/`ToolScope`/`PromptSource`/`ModelSpec`; loads built-ins from `crate::agent::registry::agents` and user TOML from the workspace/home `agents/` directory. |
| `fork_context.rs`, `sandbox_context.rs`, `spawn_depth_context.rs`, `task_recency_context.rs` | Transitional task-locals for legacy callers. Live TinyAgents tool dispatch receives `OpenHumanRunContext` directly for parent, attachments, dispatch, and usage. |
| `graph.rs` | `run_channel_turn_via_graph` (`pub(crate)`) — the channel/CLI turn graph, thin over `run_turn_via_tinyagents_shared`; called by the `agent.run_turn` native-bus handler in `agent/bus.rs`. |
| `agent_graph.rs` | `AgentGraph` (`Default`/`Custom`), `AgentTurnRequest`, `AgentTurnResult`, `AgentTurnUsage` — per-agent sub-agent turn-graph selection consulted by `subagent_runner`'s `run_typed_mode`. Every built-in agent currently selects `Default`. |
| `archivist/` | `ArchivistHook` (`types.rs`, `PostTurnHook` impl in `hook_impl.rs`) — post-turn episodic insert, segment boundary detection + lifecycle, LLM recap with heuristic fallback, lesson extraction from tool failures, and raw-prose ingestion into the memory tree when `config.learning.chat_to_tree_enabled` (`boundary.rs`, `lifecycle.rs`, `recap.rs`, `resummarise.rs`, `store.rs`, `tree_ingest.rs`, `events_heuristic.rs`). |
| `artifact_offload/` | The `outputs/` / `workspace/` convention under `action_dir`: prompt half (`contract.rs`) and host policy half (`policy.rs`); mechanics (thresholds, path resolution, pointer rendering, the writer) live in `tinyagents_harness::artifacts` and are re-exported here. |
| `run_queue/` | `RunQueue` and `QueuedMessage` — steer/followup/collect lanes wrapping `tinyagents_harness::run_queue`; `QueueStatus` is re-exported from the crate. |
| `tool_result_artifacts/` | Persists oversized individual and aggregate tool outputs under `action_dir/artifacts/tool-results/`, replacing them with a bounded `[tool_result_preview]` envelope pointing at the full, redacted file. |
| `memory_context.rs`, `memory_context_safety.rs`, `memory_protocol.rs` | Working-memory and `[Cross-chat context]` lines surfaced into the prompt (capped by `WORKING_MEMORY_LIMIT`); trust-tier wrapping of recalled entries that came from connectors (`wrap_untrusted_for_agent`); and the read-index → dedupe → write → update-index enforcement state machine for memory-mutating tools (issue #4116). |
| `instructions.rs`, `parse.rs`, `required_output.rs` | Text-mode `<tool_call>` protocol section (`build_tool_instructions*`), `parse_tool_calls_with_pformat`, and required structured-output validation/repair. |
| `credentials.rs` | `scrub_credentials` — regex scrubbing of credential-shaped text (key/value secrets, AWS access-key IDs, `sk-…` keys). Applied to every tool result by the middleware in `agent/tinyagents/`. |
| `tinyagents/host/run_context.rs` | Explicit turn-scoped dispatch refusal and `LastTurnUsage`/`SubagentUsageEntry` accounting shared by synchronous descendants. Detached work resets these turn-only handles. |

## Public surface

- `OpenHumanSessionHost`, `SessionHostBuilder`, `TurnOverrides` — re-exported from `session_host`; the
  entry point for any chat turn. External callers import these from
  `crate::agent`, which re-exports them from `session_host`.
- `run_subagent`, `SubagentRunOptions`, `SubagentRunError` — hierarchical
  sub-agent dispatch from a parent tool loop.
- `AgentDefinition`, `AgentDefinitionRegistry`, `DefinitionSource`,
  `ModelSpec`, `PromptSource`, `SandboxMode`, `ToolScope`,
  `TriggerMemoryAgent` — sub-agent archetype data model.
- `ParentExecutionContext` and its accessors (`current_parent`,
  `with_parent_context`, `current_agent_context_prepared_sources`,
  `with_agent_context_prepared_sources`) — parent runtime context for
  spawned tools.
- `current_sandbox_mode`/`with_current_sandbox_mode`,
  `current_task_recency_window`/`with_task_recency_window` — other
  task-local accessors.
- `AgentGraph`, `AgentTurnRequest`, `AgentTurnResult`, `AgentTurnUsage`.
- `LastTurnUsage`, `SubagentUsageEntry`.
- `artifact_offload::{ArtifactKind, OffloadedArtifact, new_artifact_offload, offload_oversized_result, render_artifact_offload_contract, ...}` — import via the `artifact_offload::` path (see Notes).
- `run_queue::{RunQueue, QueueMode, QueuedMessage, QueueStatus}`.

## Dependencies

- `tinyagents_harness` (vendored via `vendor/tinyagents/`) — the tool loop
  itself (`run_turn_via_tinyagents_shared`), the `run_queue` and `artifacts`
  primitives this module wraps, and the `Store` trait implemented by
  `tool_result_artifacts::ToolResultArtifactIndexStore`.
- `crate::agent::registry::agents` — built-in agent archetype TOML/prompt
  bundles loaded by `definition_loader`/`builtin_definitions`.
- `crate::security::SecurityPolicy` — workspace containment policy plumbed
  into `artifact_offload::new_artifact_offload`.
- `crate::memory` — context injection (`memory_context*`) and text
  sanitization (`tool_result_artifacts` uses `memory::safety::sanitize_text`).
- `crate::config::AgentConfig`, `crate::skills::Workflow`,
  `crate::tools::{Tool, ToolSpec}` — runtime context carried through
  `fork_context::ParentExecutionContext`.

## Used by

- `agent/mod.rs` re-exports `Agent`/`AgentBuilder` for the rest of the
  crate.
- `agent/bus.rs` serves the `agent.run_turn` native request through
  `run_channel_turn_via_graph`; channels reach the harness through that bus.
- `cron/scheduler/agent_run.rs`, `web_chat/`, `inference/local/ops/agent_chat.rs`
  (`agent_chat`) build and drive `Agent` turns directly;
  `channels/runtime/dispatch/routing.rs` consults `AgentDefinitionRegistry`/
  `ToolScope`.
- `agent/tinyagents/` middleware calls `credentials::scrub_credentials` on
  every tool result.
- `agent/orchestration/tools/*` (`spawn_subagent`, `spawn_parallel_agents`,
  `spawn_async_subagent`, `continue_subagent`, `steer_subagent`, …) and
  `agent/task_dispatcher/executor.rs` call into `subagent_runner` and the
  task-local context modules.

## Tests

- Unit: `harness_tests.rs`, `harness_gap_tests.rs`, plus `*_tests.rs` files
  beside each sub-module (`session/session_tests*.rs`,
  `subagent_runner/{ops_tests*,handoff_tests,extract_tool_tests,tool_prep_tests}.rs`,
  `run_queue/run_queue_tests.rs`, `tool_result_artifacts/mod_tests.rs`,
  `artifact_offload/artifact_offload_tests.rs`).
- Integration: `tests/agent_harness_public.rs`, `tests/agent_harness_e2e.rs`.

## Notes / gotchas

- `session/mod.rs` cites `docs/tinyagents-harness-migration-audit.md`, and
  `artifact_offload/mod.rs`, `agent_graph.rs`, `subagent_runner/ops/runner.rs`
  cite `plan-agents.md` (`docs/specs/plan-agents.md`) as the plan for moving
  durable state, the sub-agent graph, and offload mechanics onto TinyAgents
  primitives. Neither file is checked into this repo; the plan is not
  documented here.
- `artifact_offload` deliberately has no flat `ArtifactKind` re-export at the
  `harness` level — it would shadow `agent::artifacts::ArtifactKind` for glob
  importers. Use the `artifact_offload::` path.
- `run_queue::RunQueue::push` logs and drops `QueueMode::Interrupt` and
  `QueueMode::Parallel` messages: interrupts and forked turns are handled at
  the caller, never queued.

Related: [`gitbooks/developing/architecture/agent-harness.md`](../../../../../gitbooks/developing/architecture/agent-harness.md).
