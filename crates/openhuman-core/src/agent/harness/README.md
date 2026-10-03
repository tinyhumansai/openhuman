# harness

Product shell around the tinyagents tool loop. The model/tool iteration
itself runs in `crate::agent::tinyagents` (via
`run_turn_via_tinyagents_shared`); this module owns everything OpenHuman
layers on top: sub-agent definitions, parent/child context plumbing, post-turn
memory/archival hooks, and oversized-tool-result handling.

`Agent`, its per-turn lifecycle, and transcript persistence live in
`../session_host/`, not here; this module supplies the definition/prompt data
that session host and `../subagent_host/` build turns from.

Cancellation is the tinyagents steering channel (`SteeringCommand` in
`crate::agent::tinyagents`); there is no in-house interrupt fence or
cancellation token owned here. The message queue that lets a caller steer or
follow up on an in-flight turn is `tinyagents_harness::run_queue::RunQueue`,
used here (not defined here) by `agent_graph.rs` and `fork_context.rs`.

## Responsibilities

- Define sub-agent archetypes, built-in and workspace TOML, and the
  definition/prompt inputs consumed by `../session_host/` (which owns the
  `Agent` struct, turn lifecycle, and KV-cache prefix stability) and
  `../subagent_host/`, which implements the TinyAgents sub-agent lifecycle
  traits directly. This module does not export a compatibility runner of its
  own.
- Carry the task-local plumbing that lets a spawned tool see its parent's
  runtime context (`fork_context.rs`, `sandbox_context.rs`,
  `spawn_depth_context.rs`, `task_recency_context.rs`, `OpenHumanRunContext`).
- Run the channel/CLI turn graph (`graph.rs`) and let a built-in agent select
  a bespoke sub-agent turn graph (`agent_graph.rs`).
- Offload oversized worker artifacts to the filesystem, and persist oversized
  tool results as action-workspace artifacts (`artifact_offload/`,
  `tool_result_artifacts/`).

## Key sub-modules

| Module | Role |
| --- | --- |
| `definition*.rs`, `builtin_definitions.rs`, `definition_loader.rs` | `AgentDefinition`/`AgentDefinitionRegistry`/`SandboxMode`/`ToolScope`/`PromptSource`/`ModelSpec`; loads built-ins from `crate::agent::registry::agents` and user TOML from the workspace/home `agents/` directory. |
| `fork_context.rs`, `sandbox_context.rs`, `spawn_depth_context.rs`, `task_recency_context.rs` | Task-locals that let a spawned tool see its parent's runtime context: parent handle, sandbox mode, spawn depth, and task-recency window. `fork_context.rs` also carries the `RunQueue` handle (from `tinyagents_harness::run_queue`) down to a forked turn. |
| `graph.rs` | `run_channel_turn_via_graph` (`pub(crate)`): the channel/CLI turn graph, thin over `run_turn_via_tinyagents_shared`; called by the `agent.run_turn` native-bus handler in `agent/bus.rs`. |
| `agent_graph.rs` | `AgentGraph` (`Default`/`Custom`), `AgentTurnRequest`, `AgentTurnResult`, `AgentTurnUsage`: per-agent sub-agent turn-graph selection consumed by `../subagent_host/`. Every built-in agent currently selects `Default`. |
| `artifact_offload/` | The `outputs/` / `workspace/` convention under `action_dir`: prompt half (`contract.rs`) and host policy half (`policy.rs`); mechanics (thresholds, path resolution, pointer rendering, the writer) live in `tinyagents_harness::artifacts` and are re-exported here. |
| `tool_result_artifacts/` | Wiring only: `new_tool_result_store` hands `tinyagents_harness::artifacts::tool_results` OpenHuman's redactor (`SanitizingRedactor`), `file_read`/`use_skill` names and `FileReadTool::MAX_FILE_SIZE_BYTES`. The store, `[tool_result_preview]` envelope, budgets and paged reads live in the crate. |
| `memory_context_safety.rs` | Trust-tier wrapping of recalled entries that came from connectors (`wrap_untrusted_for_agent`). The read-index, dedupe, write, update-index enforcement state machine for memory-mutating tools (issue #4116) now lives in `tinyagents_harness::middleware` (`MemoryProtocolTracker`). |
| `required_output.rs` | Pure validate/repair/synthesize primitives (issue #4117) that guarantee a required structured-output block (for example a `thoughts` JSON block) on every accepted turn. The orchestration that calls these lives on the session in `../session_host/turn/`. |
| `credentials.rs` | `scrub_credentials`: regex scrubbing of credential-shaped text (key/value secrets, AWS access-key IDs, `sk-...` keys). Applied to every tool result by `CredentialScrubMiddleware` in `agent/tinyagents/middleware/credential_scrub.rs`, installed as the innermost tool wrap so nothing downstream sees the raw secret. |

## Public surface

What `harness/mod.rs` actually re-exports:

- `AgentDefinition`, `AgentDefinitionRegistry`, `DefinitionSource`,
  `ModelSpec`, `PromptSource`, `SandboxMode`, `ToolScope`: the sub-agent
  archetype data model.
- `ParentExecutionContext` and its accessors (`current_parent`,
  `with_parent_context`, `AgentContextPreparedSource`):
  parent runtime context for spawned tools.
- `current_sandbox_mode`/`with_current_sandbox_mode`,
  `current_task_recency_window`/`with_task_recency_window`: the other
  task-local accessors.
- `AgentGraph`, `AgentTurnRequest`, `AgentTurnResult`, `AgentTurnUsage`.
- `artifact_offload::{...}`: reached only via the `artifact_offload::` path
  (see Notes), never flattened into `harness::`.

Adjacent public surface that lives in sibling modules, not here:
`OpenHumanSessionHost`/`SessionHostBuilder`/`TurnOverrides` and the `Agent`
turn lifecycle are in `../session_host/`; `run_subagent`,
`SubagentRunOptions`, `SubagentRunError` are in `../subagent_host/`;
`LastTurnUsage`/`SubagentUsageEntry` are in `../tinyagents/host/run_context.rs`.

## Dependencies

- `tinyagents_harness` (vendored via `vendor/tinyagents/`): the tool loop
  itself (`run_turn_via_tinyagents_shared`), the `run_queue` and `artifacts`
  primitives this module wraps, and the `InMemoryStore` used as the
  tool-result artifact index.
- `tinytools_agent` (also vendored): the tool-call parsers themselves
  (`<tool_call>` tags, fenced blocks, bare JSON, `<invoke>` XML, GLM grammar,
  p-format). This crate is a test-only dependency of `harness/`; the parsers
  are not called from production code here.
- `crate::agent::registry::agents`: built-in agent archetype TOML/prompt
  bundles loaded by `definition_loader`/`builtin_definitions`.
- `crate::security::SecurityPolicy`: workspace containment policy plumbed
  into `artifact_offload::new_artifact_offload`.
- `crate::memory`: trust-tier wrapping (`memory_context_safety`) and text
  sanitization (`artifact_offload::SanitizingRedactor` wraps `memory::safety::sanitize_text` for both artifact stores).
- `crate::config::AgentConfig`, `crate::skills::Workflow`,
  `crate::tools::{Tool, ToolSpec}`: runtime context carried through
  `fork_context::ParentExecutionContext`.

## Used by

- `../session_host/` and `../subagent_host/` build turns from the
  definitions, prompt data, and task-local context this module supplies.
- `agent/bus.rs` serves the `agent.run_turn` native request through
  `run_channel_turn_via_graph`; channels reach the harness through that bus.
- `channels/runtime/dispatch/routing.rs` consults
  `AgentDefinitionRegistry`/`ToolScope`.
- `agent/tinyagents/middleware/credential_scrub.rs` calls
  `harness::credentials::scrub_credentials` on every tool result.
- `agent/orchestration/tools/*` (`spawn_subagent`, `spawn_parallel_agents`,
  `spawn_async_subagent`, `continue_subagent`, `steer_subagent`, and friends)
  call into `../subagent_host/` and this module's task-local context
  functions.

## Tests

- Unit: `harness_tool_call_parsing_edge_case_tests.rs`, plus `*_tests.rs` files
  beside each sub-module (`tool_result_artifacts/mod_tests.rs`,
  `artifact_offload/artifact_offload_tests.rs`).
- Integration: `tests/in_process/agent_harness_public.rs`, `tests/agent_harness_e2e.rs`.

## Notes / gotchas

- `agent_graph.rs`, `artifact_offload/mod.rs`, and
  `../subagent_host/ops/runner.rs` cite `docs/specs/plan-agents.md` as the
  plan for moving durable state, the sub-agent graph, and offload mechanics
  onto TinyAgents primitives. That file is not checked into this repo, so the
  plan itself is not documented here, only the citations to it.
- `artifact_offload` deliberately has no flat `ArtifactKind` re-export at the
  `harness` level. It would shadow `agent::artifacts::ArtifactKind` for glob
  importers, so use the `artifact_offload::` path instead.
- The `Agent` struct, its turn lifecycle, and transcript persistence used to
  live under `harness/session/`. That code has moved to `../session_host/`;
  this README describes only what remains in `harness/` today.

Related: [`gitbooks/developing/architecture/agent-harness.md`](../../../../../gitbooks/developing/architecture/agent-harness.md).
