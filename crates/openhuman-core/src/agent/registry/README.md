# registry

User-facing agent registry. Owns the `agent_registry` RPC namespace: the
merged list of shipped default agents and user-authored custom agents, their
enable/disable state, and their tool visibility policy. This is a config
layer over the harness — the tool-calling loop and prompt/runtime
implementation live in [`agent/harness`](../harness/), and the definitions
the harness runs come from [`agents/`](agents/) below.

## Files

- [`types.rs`](types.rs) — wire types: `AgentRegistryConfig` (the persisted
  `entries: Vec<AgentRegistryEntry>`, stored at `Config.agent_registry`),
  `AgentRegistryEntry` (id, name, description, `source: AgentRegistrySource`
  — `Default` vs `Custom` — enabled, model, optional system prompt,
  `tool_allowlist`/`tool_denylist`, `subagents: AgentSubagentPolicy`
  allowlist, tags, free-form metadata), `AgentRegistryPatch` (partial
  update), `AgentToolInfo` (tool-picker row). `AgentSubagentPolicy` also
  deserializes from a legacy bare list of ids. `AgentRegistryEntry::validate`
  enforces non-blank id/name/description and an ASCII `[A-Za-z0-9_-]` id
  charset for both the entry and its subagent allowlist.
- [`defaults.rs`](defaults.rs) — `default_agents()` loads the built-ins from
  [`agents::load_builtins`](agents/loader.rs) and maps each `AgentDefinition`
  to a `Default`-sourced registry entry (tier becomes the single tag,
  `ToolScope::Wildcard` renders as `["*"]`). `definition_from_registry_entry`
  is the inverse: it synthesizes an `AgentDefinition` from a *custom* entry
  (one with no shipped harness definition) so a custom agent runs through the
  same `Agent::from_config_for_agent` factory path as a built-in, with a real
  tool belt instead of a persona-only completion. Every harness-only field
  (`omit_*`, temperature, iteration policy, sandbox mode, tier) takes the
  harness's own safe worker default; see the function's doc comment for the
  exact mapping and why an empty `tool_allowlist` must stay `Named(vec![])`
  rather than collapsing to `Wildcard`.
- [`ops.rs`](ops.rs) — config-backed CRUD: `list_agents`, `get_agent`,
  `upsert_custom_agent` (rejects ids that collide with a default),
  `update_agent` (copies the default into config on first edit; refuses to
  disable `orchestrator`), `set_agent_enabled`, `remove_agent`,
  `merge_entries` (defaults overlaid with persisted config entries,
  default-first), `available_tools`, `find_custom_in_config` (synchronous,
  matches only *enabled* `Custom` entries — used by the agent factory on a
  harness-registry miss). Reads/writes through
  `config::rpc::load_config_with_timeout` and `Config::save`. Tool listing
  (`available_tools`) builds the `tools_agent` built-in and reads its
  `tool_specs()` — its tool scope is the full catalog, unlike the
  orchestrator's curated subset.
- [`schemas.rs`](schemas.rs) — `ControllerSchema`/`RegisteredController`
  definitions for namespace `agent_registry`: `list`, `get`,
  `available_tools`, `create_custom`, `upsert_custom`, `update`,
  `set_enabled`, `remove`. Registered through
  `all_agent_registry_registered_controllers` in `core/all.rs`.
- [`rpc.rs`](rpc.rs) — request/response payload types and the `*_rpc` handler
  functions schemas.rs wires up, delegating into `ops.rs`.
- [`tools.rs`](tools.rs) — backwards-compatible re-export of
  `agent::orchestration::tools::*`; nothing in the tree imports it any more.
- [`agents/`](agents/) — the built-in agent archetypes and their loader.

## `agents/`

Each built-in agent owns a subfolder with an `agent.toml` (id, `when_to_use`,
model, tool scope, sandbox mode, iteration cap, tier, `omit_*` flags —
parsed directly into `AgentDefinition`), a `prompt.md` holding the static
archetype body, and a `prompt.rs` that `include_str!`s that body and exposes
`pub fn build(&PromptContext) -> anyhow::Result<String>`, appending
runtime-dependent sections (rendered tool list, user files, workspace) to it. `researcher` additionally
owns a `graph.rs` for a bespoke `AgentGraph`; every other archetype uses
`AgentGraph::Default`. The per-archetype contract is documented on
[`agents/mod.rs`](agents/mod.rs).

[`agents/loader.rs`](agents/loader.rs) owns the `BUILTINS` slice and
`load_builtins`, which parses each `agent.toml`, installs
`PromptSource::Dynamic(prompt_fn)` and the optional graph, stamps
`DefinitionSource::Builtin`, checks that the folder id matches the TOML id,
and then runs `validate_tier_hierarchy`: a `worker` may not list any agent-id
subagent, and `chat -> chat` / `reasoning -> reasoning` delegation is
rejected (the pair rule is `validate_tier_transition` in the harness; unknown
subagent ids are tolerated here as a separate integrity concern).
`BUILTINS` is also the registration point for archetypes that live with
other domains: `agent_memory` (`memory/agent/agent/`), `skill_setup` and
`skill_executor` (`skills/{catalog,runtime}/agent/`, feature `skills`), and
`workflow_builder` and `flow_discovery` (`flows/agents/`, feature `flows`).
Workspace-level overrides (`<workspace_dir>/agents/*.toml`, with a
`~/.openhuman/agents/` fallback) are loaded separately by
`agent::harness::definition_loader` and replace built-ins on id collision;
`AgentDefinitionRegistry::load` re-runs `validate_tier_hierarchy` after that
merge.

The 29 archetypes in this directory:

| Archetype | Role |
| --- | --- |
| `archivist` | Background: extracts lessons from a completed session into `MEMORY.md` and FTS5 |
| `code_executor` | Repo-scoped worker: locate/read/edit/build/test/git for any repo work |
| `context_scout` | Read-only pre-flight context bundle (memory, goals, integrations, web) |
| `critic` | Adversarial, read-only reviewer of diffs/code against project rules |
| `crypto_agent` | Wallet/market specialist: balances, swaps, contract calls, x402 paid requests |
| `flow_memory_agent` (feature `flows`) | Read-only context/memory retrieval for automation-flow `agent` nodes |
| `goals_agent` | Background: keeps `MEMORY_GOALS.md` fresh from session context |
| `help` | Answers "how does OpenHuman work" questions from the bundled GitBook docs |
| `image_agent` | Image generation/edit specialist |
| `integrations_agent` | Drives a single Composio toolkit (gmail, notion, github, …) per spawn |
| `mcp_agent` (feature `mcp`) | Calls tools on an already-connected MCP server |
| `mcp_setup` | Walks the user through installing/connecting a new MCP server |
| `morning_briefing` | Proactive scheduled daily summary (tasks, calendar, email, skills) |
| `orchestrator` | Default user-facing `chat`-tier agent; direct-first, delegates only when it materially helps |
| `planner` | Read-only `reasoning`-tier architect: breaks a task into a DAG of subtasks with acceptance criteria |
| `presentation_agent` (feature `documents`) | Builds decks from evidence; owns grounding/citations/image verification |
| `profile_memory_agent` | Profile, persona, preferences, people-graph specialist |
| `researcher` | Web/docs crawler that compresses findings to dense markdown; has a custom `graph.rs` |
| `scheduler_agent` | Reminders, recurring jobs, cron — time/cron tools only, no live calendar reads |
| `settings_agent` | App/core config, health/model diagnostics, service lifecycle, security policy |
| `skill_creator` | Creates/updates SKILL.md packages and Node-backed JS helpers |
| `summarizer` | Runtime-dispatched only: compresses oversized tool results for the orchestrator |
| `task_manager_agent` | Task-board/task-source specialist: cards, feeds, artifacts, status |
| `tool_maker` | Narrow self-healer: writes a polyfill when a host command is missing |
| `tools_agent` | Generalist heavy execution (shell/HTTP/web/files) that never touches a repo or git; wildcard tool scope |
| `trigger_reactor` | One or two tool calls in direct reaction to an external trigger, no planning |
| `trigger_triage` | Classifies an external trigger into drop/acknowledge/react/escalate; never acts |
| `video_agent` | Video generation/animation specialist |
| `vision_agent` | Read-only image understanding: describe, OCR, locate UI elements |

`flow_memory_agent` and `mcp_agent` are `#[cfg]`-gated out of both the
module list and `BUILTINS` when `flows`/`mcp` is disabled;
`presentation_agent` stays compiled but `builtin_enabled` drops it from
`load_builtins` without `documents`, in lockstep with its
`generate_presentation` tool. The orchestrator's `agent.toml` subagent list
still names `mcp_agent` unconditionally (TOML can't be `cfg`'d); both the
orchestrator tool synthesis in `tools/orchestrator_tools.rs` and
`validate_tier_hierarchy` skip that dangling id rather than failing boot.

## Called by

- `core/all.rs` — registers the `agent_registry` controllers under
  `DomainGroup::Agent`.
- `config/schema/` — `Config.agent_registry: AgentRegistryConfig` is the
  persisted store every `ops.rs` function reads and writes.
- `agent/harness/builtin_definitions.rs` — `load_builtins()` seeds the
  process-global `AgentDefinitionRegistry`; `agent/harness/definition/registry.rs`
  calls `validate_tier_hierarchy` again after workspace overrides merge.
- `agent/session_host/builder/factory.rs` —
  `Agent::from_config_for_agent` falls back to `find_custom_in_config` +
  `definition_from_registry_entry` when an id is not in the harness registry.
- `agent/schemas.rs` — `agent.graph_topologies` and `agent.registry_snapshot`
  enumerate `load_builtins()`.
- `flows/` (`ops/inference_readiness.rs`, `ops/builder_gates.rs`,
  `builder_tools/kind_reads.rs`, `tinyflows/caps/agent.rs`) — resolve a flow
  `agent` node's `agent_ref` through `list_agents`/`get_agent`/`find_custom_in_config`.

## Tests

`defaults_tests.rs`, `ops_tests.rs`, `schemas_tests.rs`, `types_tests.rs`;
under `agents/`: `loader_tests.rs` (+ `loader_tests_orchestrator_tier_tests.rs`,
`loader_tests_builtin_registration_tests.rs`, `loader_tests_specialist_agents_tests.rs`)
and a `prompt_tests.rs` beside almost every archetype's `prompt.rs`.
