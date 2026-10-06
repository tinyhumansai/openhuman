# registry

User-facing agent registry. Owns the `agent_registry` RPC namespace: the
merged list of shipped default agents and user-authored custom agents, their
enable/disable state, and their tool visibility policy. This is a config
layer over the harness: the tool-calling loop and prompt/runtime
implementation live in [`agent/harness`](../harness/), and the definitions
the harness runs come from [`agents/`](agents/) below.

## Files

- [`types.rs`](types.rs): wire types. `AgentRegistryConfig` (the persisted
  `entries: Vec<AgentRegistryEntry>`, stored at `Config.agent_registry`),
  `AgentRegistryEntry` (id, name, description, `source: AgentRegistrySource`,
  `Default` vs `Custom`, enabled, model, optional system prompt,
  `tool_allowlist`/`tool_denylist`, `subagents: AgentSubagentPolicy`
  allowlist, tags, free-form metadata), `AgentRegistryPatch` (partial
  update), `AgentToolInfo` (tool-picker row). `AgentSubagentPolicy` also
  deserializes from a legacy bare list of ids. `AgentRegistryEntry::validate`
  enforces non-blank id/name/description and an ASCII `[A-Za-z0-9_-]` id
  charset for both the entry and its subagent allowlist.
- [`defaults.rs`](defaults.rs): `default_agents()` loads the built-ins from
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
- [`ops.rs`](ops.rs): config-backed CRUD. `list_agents`, `get_agent`,
  `upsert_custom_agent` (rejects ids that collide with a default),
  `update_agent` (copies the default into config on first edit; refuses to
  disable `orchestrator`), `set_agent_enabled`, `remove_agent`,
  `merge_entries` (defaults overlaid with persisted config entries,
  default-first), `available_tools`, `find_custom_in_config` (synchronous,
  matches only *enabled* `Custom` entries, used by the agent factory on a
  harness-registry miss). It reads and writes through
  `config::rpc::load_config_with_timeout` and `Config::save`. Tool listing
  (`available_tools`) builds an `orchestrator` session and reads its durable
  tool registry (`durable_tool_specs_arc()`), so it lists every registered
  tool, including `Deferred` ones the orchestrator reaches through
  `tool_search` and packed ones behind `use_skill`, not only the schemas it
  advertises per turn.
- [`schemas.rs`](schemas.rs): `ControllerSchema`/`RegisteredController`
  definitions for namespace `agent_registry`: `list`, `get`,
  `available_tools`, `create_custom`, `upsert_custom`, `update`,
  `set_enabled`, `remove`. Registered through
  `all_agent_registry_registered_controllers` in `core/all.rs`.
- [`rpc.rs`](rpc.rs): request/response payload types and the `*_rpc` handler
  functions that `schemas.rs` wires up, delegating into `ops.rs`.
- [`tools.rs`](tools.rs): backwards-compatible re-export of
  `agent::orchestration::tools::*`; nothing in the tree imports it any more.
- [`agents/`](agents/): the built-in agent archetypes and their loader.

## `agents/`

Each built-in agent owns a subfolder with an `agent.toml` (id, `when_to_use`,
model, tool scope, sandbox mode, iteration cap, tier, `omit_*` flags, parsed
directly into `AgentDefinition`), a `prompt.md` holding the static archetype
body, and a `prompt.rs` that `include_str!`s that body and exposes
`pub fn build(&PromptContext) -> anyhow::Result<String>`, appending
runtime-dependent sections (rendered tool list, workspace) to it.
Every archetype currently uses `AgentGraph::Default`; an archetype that needs a
bespoke `AgentGraph` adds a `graph.rs` and sets `BuiltinAgent::graph_fn`. The per-archetype contract is
documented on [`agents/mod.rs`](agents/mod.rs).

[`agents/loader.rs`](agents/loader.rs) owns the `BUILTINS` slice and
`load_builtins`, which parses each `agent.toml`, installs
`PromptSource::Dynamic(prompt_fn)` and the optional graph, stamps
`DefinitionSource::Builtin`, checks that the folder id matches the TOML id,
and then runs `validate_tier_hierarchy`: a `worker` may not list any agent-id
subagent, and `chat -> chat` / `reasoning -> reasoning` delegation is
rejected (the pair rule is `validate_tier_transition` in the harness; unknown
subagent ids are tolerated here as a separate integrity concern).
`BUILTINS` is also the registration point for archetypes that live with
other domains: `skill_setup`
(`skills/catalog/agent/`, feature `skills`), and
`workflow_builder` and `flow_discovery` (`flows/agents/`, feature `flows`).
Workspace-level overrides (`<workspace_dir>/agents/*.toml`, with a
`~/.openhuman/agents/` fallback) are loaded separately by
`agent::harness::definition_loader` and replace built-ins on id collision;
`AgentDefinitionRegistry::load` re-runs `validate_tier_hierarchy` after that
merge.

Config-level edits (`config.agent_registry.entries`: `agent_registry_update`
on a shipped agent saves a `Default`-sourced copy with the patch applied;
`agent_registry_create_custom` saves a `Custom` entry) are **not** merged into
`AgentDefinitionRegistry` — it is loaded once per process. The runtime reads
them through `effective.rs` instead, from the `Config` snapshot a session is
built with, so they take effect for sessions built after the save without a
restart:

- `effective_subagent_allowlist`: a saved override's `subagents.allowlist`
  replaces the shipped list (only when it differs from it, so an edit that
  touched something else does not freeze the allowlist at that snapshot) in
  the `spawn_async_subagent` enum, the scoped tool instance, and the parent's
  execute-side gate (`ParentExecutionContext::allowed_subagent_ids`).
- `resolve_spawnable_definition` / `spawnable_ids`: every spawn path resolves
  a child id against the harness registry first, then an enabled `Custom`
  entry via `definition_from_registry_entry`, so a user-authored sub-agent is
  dispatchable once the parent's allowlist names it (#6934). The parent's
  config travels on `ParentExecutionContext::runtime_config`; a parent built
  without one resolves against the harness registry alone.

The 12 archetypes in this directory:

| Archetype | Role |
| --- | --- |
| `critic` | Workflow-run worker: adversarial, read-only cross-check of claims, diffs and code. Not a chat delegate |
| `image_agent` | Image generation/edit specialist |
| `morning_briefing` | Proactive scheduled daily summary (tasks, calendar, email, skills) on a named, read-only tool belt |
| `orchestrator` | Default user-facing `chat`-tier agent; direct-first, delegates only when it materially helps |
| `planner` | Workflow-run worker (`reasoning` tier): decomposes a question into research angles, or researches one angle. Read-only; not a chat delegate |
| `presentation_agent` (feature `documents`) | Builds decks from evidence; owns grounding/citations/image verification |
| `summarizer` | Runtime-dispatched only: compresses oversized tool results for the orchestrator, and synthesizes workflow-run reports |
| `task_manager_agent` | Task-source/workflow/artifact specialist: proactive feeds, workflow bundles, artifacts |
| `trigger_reactor` | One or two tool calls in direct reaction to an external trigger, no planning |
| `trigger_triage` | Classifies an external trigger into drop/acknowledge/react/escalate; never acts |
| `video_agent` | Video generation/animation specialist |
| `vision_agent` | Read-only image understanding: describe, OCR, locate UI elements |

The orchestrator's chat delegates (its `[subagents]` allowlist) are
`task_manager_agent`, `vision_agent`,
`image_agent`, `video_agent`, `presentation_agent`, `skill_setup`,
`workflow_builder` and `flow_discovery`. `planner` and `critic` stay
registered only for the `parallel_research_cross_check` workflow-run template
(`agent/orchestration/workflow_runs/ops.rs`: decompose = `planner`, research
= `planner` x2, cross_check = `critic`, synthesize = `summarizer`); the
orchestrator does not list them. Everything else is runtime-only.

The single-belt specialists that used to sit here (`code_executor`,
`crypto_agent`, `settings_agent`, `scheduler_agent`, `mcp_agent`,
`tools_agent`, `help`, `tool_maker`, `skill_creator`, `context_scout`,
`integrations_agent`, `skill_executor`, `researcher`) were removed. Their
playbooks are now inline skill guides on tool packs (`coding`, `web3`,
`system`, `scheduling`, `docs`, `mcp`) that the orchestrator loads with
`use_skill`, and their tools are `Deferred`, so `tool_search` finds them
too; see [`tools/toolpacks/README.md`](../../tools/toolpacks/README.md#inline-skill-guides).
Running an installed skill is the orchestrator's own `run_workflow`.

`presentation_agent` stays compiled but
`builtin_enabled` drops it from `load_builtins` without `documents`, in
lockstep with its `generate_presentation` tool.

The orchestrator picks tools at request time through the shared tool-search
ranker (Jev when a decision-model credential is installed, BM25 otherwise);
see [`tinyagents/README.md`](../tinyagents/README.md) and
[the Jev page](../../../../../gitbooks/developing/jev.md) for how that
ranking works.

## Called by

- `core/all.rs`: registers the `agent_registry` controllers under
  `DomainGroup::Agent`.
- `config/schema/`: `Config.agent_registry: AgentRegistryConfig` is the
  persisted store every `ops.rs` function reads and writes.
- `agent/harness/builtin_definitions.rs`: `load_builtins()` seeds the
  process-global `AgentDefinitionRegistry`; `agent/harness/definition/registry.rs`
  calls `validate_tier_hierarchy` again after workspace overrides merge.
- `agent/session_host/builder/factory.rs`:
  `Agent::from_config_for_agent` falls back to `find_custom_in_config` +
  `definition_from_registry_entry` when an id is not in the harness registry.
- `agent/schemas.rs`: `agent.graph_topologies` and `agent.registry_snapshot`
  enumerate `load_builtins()`.
- `flows/` (`ops/inference_readiness.rs`, `ops/builder_gates.rs`,
  `builder_tools/kind_reads.rs`, `tinyflows/caps/agent.rs`): resolve a flow
  `agent` node's `agent_ref` through `list_agents`/`get_agent`/`find_custom_in_config`.

## Tests

`defaults_tests.rs`, `ops_tests.rs`, `schemas_tests.rs`, `types_tests.rs`;
under `agents/`: `loader_tests.rs` (+ `loader_tests_orchestrator_tier_tests.rs`,
`loader_tests_builtin_registration_tests.rs`, `loader_tests_specialist_agents_tests.rs`)
and a `prompt_tests.rs` beside almost every archetype's `prompt.rs`.
