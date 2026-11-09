# harness

Product shell around the tinyagents tool loop. The model/tool iteration
itself runs in `crate::agent::tinyagents` (via
`TurnSpec::run`). This module owns sub-agent definitions, post-turn
memory/archival hooks, and oversized-tool-result handling. The channel/CLI
turn driver lives with channel runtime at
[`../../channels/runtime/turn_driver.rs`](../../channels/runtime/turn_driver.rs).
Parent execution snapshots live with the sub-agent host in
[`../subagent_host/fork_context.rs`](../subagent_host/fork_context.rs).

`Agent`, its per-turn lifecycle, and transcript persistence live in
[`../session_host/`](../session_host/), not here; this module supplies the definition/prompt data
that session host and [`../subagent_host/`](../subagent_host/) build turns from.

Cancellation is the tinyagents steering channel (`SteeringCommand` in
`crate::agent::tinyagents`); there is no in-house interrupt fence or
cancellation token owned here. The message queue that lets a caller steer or
follow up on an in-flight turn is `tinyagents_harness::run_queue::RunQueue`,
carried by the parent snapshot in [`../subagent_host/fork_context.rs`](../subagent_host/fork_context.rs).

## Responsibilities

- Define sub-agent archetypes, built-in and workspace TOML, and the
  definition/prompt inputs consumed by `../session_host/` (which owns the
  `Agent` struct, turn lifecycle, and KV-cache prefix stability) and
  `../subagent_host/`, which implements the TinyAgents sub-agent lifecycle
  traits directly. This module does not export a compatibility runner of its
  own.
- Offload oversized worker artifacts to the filesystem, and persist oversized
  tool results as action-workspace artifacts (`artifact_offload/`,
  `tool_result_artifacts/`).

## Key sub-modules

| Module | Role |
| --- | --- |
| `artifact_offload/` | The `outputs/` / `workspace/` convention under `action_dir`: prompt half (`contract.rs`) and host policy half (`policy.rs`); mechanics (thresholds, path resolution, pointer rendering, the writer) live in `tinyagents_harness::artifacts` and are re-exported here. |
| `tool_result_artifacts/` | Wiring only: `new_tool_result_store` hands `tinyagents_harness::artifacts::tool_results` OpenHuman's redactor (`SanitizingRedactor`), `file_read`/`use_skill` names and `FileReadTool::MAX_FILE_SIZE_BYTES`. The store, `[tool_result_preview]` envelope, budgets and paged reads live in the crate. |
| `required_output.rs` | Pure validate/repair/synthesize primitives (issue #4117) that guarantee a required structured-output block (for example a `thoughts` JSON block) on every accepted turn. The orchestration that calls these lives on the session in [`../session_host/turn/`](../session_host/turn/). |
| `credentials.rs` | `scrub_credentials`: regex scrubbing of credential-shaped text (key/value secrets, AWS access-key IDs, `sk-...` keys). Applied to every tool result by `CredentialScrubMiddleware` in [`agent/tinyagents/middleware/credential_scrub.rs`](../tinyagents/middleware/credential_scrub.rs), installed as the innermost tool wrap so nothing downstream sees the raw secret. |

## Public surface

What `harness/mod.rs` actually re-exports:

- `AgentDefinition`, `AgentDefinitionRegistry`, `DefinitionSource`,
  `ModelSpec`, `PromptSource`, `SandboxMode`, `ToolScope`: the sub-agent
  archetype data model.
Sandbox policy lives on the explicit host operation context. Briefing task
recency belongs to ``OpenHumanRunExt` (agent/tinyagents/host/run_context.rs)`.
- `artifact_offload::{...}`: reached only via the `artifact_offload::` path
  (see Notes), never flattened into `harness::`.

Adjacent public surface that lives in sibling modules, not here:
`OpenHumanSessionHost`/`SessionHostBuilder`/`TurnOverrides` and the `Agent`
turn lifecycle are in `../session_host/`; `OpenHumanSubagentHost`,
`SubagentRunOptions`, and `SubagentRunError` are in `../subagent_host/`;
`LastTurnUsage`/`SubagentUsageEntry` are in [`../tinyagents/host/run_context.rs`](../tinyagents/host/run_context.rs).

## Dependencies

- `tinyagents_harness` (vendored via [`vendor/tinyagents/`](../../../../../vendor/tinyagents/)): the tool loop
  itself (`TurnSpec::run`), the `run_queue` and `artifacts`
  primitives this module wraps, and the `InMemoryStore` used as the
  tool-result artifact index.
- `tinytools_agent` (also vendored): the tool-call parsers themselves
  (`<tool_call>` tags, fenced blocks, bare JSON, `<invoke>` XML, GLM grammar,
  p-format). This crate is a test-only dependency of `harness/`; the parsers
  are not called from production code here.
- `crate::agent_registry::agents`: built-in agent archetype TOML/prompt
  bundles; custom bundles are loaded by `tinyagents_definition`.
- `crate::security::SecurityPolicy`: workspace containment policy plumbed
  into `artifact_offload::new_artifact_offload`.
- `tinymemory-api`: provenance classification and untrusted-context wrapping.
- `crate::memory`: text
  sanitization (`artifact_offload::SanitizingRedactor` wraps `memory::safety::sanitize_text` for both artifact stores).
- `crate::config::AgentConfig`, `crate::skills::Workflow`,
  `crate::tools::{Tool, ToolSpec}`: inputs consumed by the session and
  sub-agent hosts.

## Used by

- `../session_host/` and `../subagent_host/` build turns from the
  definitions and prompt data this module supplies; parent context types are
  owned by `../subagent_host/`.
- `agent/bus.rs` serves the `agent.run_turn` native request through
  `run_channel_turn` in [`channels/runtime/turn_driver.rs`](../../channels/runtime/turn_driver.rs);
  channels reach it through that bus.
- [`channels/runtime/dispatch/routing.rs`](../../channels/runtime/dispatch/routing.rs) consults
  `AgentDefinitionRegistry`/`ToolScope`.
- `agent/tinyagents/middleware/credential_scrub.rs` calls
  `harness::credentials::scrub_credentials` on every tool result.
- `agent/orchestration/tools/*` (`spawn_subagent`)
  call into `../subagent_host/` and take its parent context explicitly.

## Tests

- Unit: `harness_tool_call_parsing_edge_case_tests.rs`, plus `*_tests.rs` files
  beside each sub-module ([`tool_result_artifacts/mod_tests.rs`](./tool_result_artifacts/mod_tests.rs),
  [`artifact_offload/artifact_offload_tests.rs`](./artifact_offload/artifact_offload_tests.rs)).
- Integration: [`tests/in_process/agent_harness_public.rs`](../../../../../tests/in_process/agent_harness_public.rs), [`tests/agent_harness_e2e.rs`](../../../../../tests/agent_harness_e2e.rs).

## Notes / gotchas

- [`artifact_offload/mod.rs`](./artifact_offload/mod.rs) and
  [`../subagent_host/ops/runner.rs`](../subagent_host/ops/runner.rs) cite `docs/specs/plan-agents.md` as the
  plan for moving durable state, the sub-agent graph, and offload mechanics
  onto TinyAgents primitives. That file is not checked into this repo, so the
  plan itself is not documented here, only the citations to it.
- `artifact_offload` deliberately has no flat `ArtifactKind` re-export at the
  `harness` level. It would shadow `artifacts::ArtifactKind` for glob
  importers, so use the `artifact_offload::` path instead.
- The `Agent` struct, its turn lifecycle, and transcript persistence used to
  live under `harness/session/`. That code has moved to `../session_host/`;
  this README describes only what remains in `harness/` today.

Related: [`docs/gitbooks/en/developing/architecture/agent-harness.md`](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md).

## Further reading

- [Parent module README](../README.md)
- [The orchestrator](../../../../../docs/gitbooks/en/features/orchestration.md)
