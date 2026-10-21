# toolpacks

On-demand tool disclosure. A tool's JSON schema is charged on every provider
call of every turn whether or not the tool is used; the Master Agent's belt
measured ~21.4k tokens of schema against a ~4.3k-token system prompt, so most
of that fixed cost is idle in most conversations. A pack keeps its tools
constructed and executable but unadvertised: the agent sees one small tool,
`use_skill`, instead of the pack's real schemas.

## How it works

- `types::ToolPack` is the unit: an `id`, a one-line `summary` shown in the
  always-on pack index, the `tools` it owns, `owners` (agent ids the pack is
  *not* applied to), because the specialist a family was delegated to
  (`skill_setup` for `skills`, `workflow_builder` for `workflows`, ...)
  should not pay a `use_skill` round trip on every call of its own belt, and
  an optional `guide` (see [Inline skill guides](#inline-skill-guides)).
- `registry::PACKS` is the compiled-in table. Membership is a build-time
  decision on purpose: a pack that config or RPC could edit would let a caller
  move a dangerous tool out of the reviewed, advertised surface.
  `pack`, `pack_for_tool`, `all_packed_tool_names`,
  `packed_tool_names_for_agent`, `pack_index_markdown[_filtered]`, and
  `callable_pack_ids` all read this table. The `pub(crate)`
  `DELIBERATELY_UNPACKED_FLEET_TOOLS` list names the delegation family that is
  kept *off* packs because it is used on almost every turn;
  `agent/orchestration/tools/collapsed_delegation.rs` collapses that family
  into `delegate_to` instead and explains the trade.
- `tools::UseSkillTool` (`USE_SKILL = "use_skill"`) is the one always-on proxy
  tool. Called with `skill` alone it renders that pack's tool schemas into the
  conversation; with `skill` + `tool` + `args` it resolves the real tool
  (refusing a tool that the named pack does not own) and forwards
  `permission_level[_with_args]`, `timeout_policy`, and
  `external_effect_with_args` to it, so nothing is laundered through the
  proxy. The tool's own `execute` path renders the pack unfiltered; the
  session-scoped listing (`render_pack_filtered` + `route_sentence`) is
  produced by the tinyagents policy middleware, which is the only layer that
  holds the session (see below).
- `tinyagents_harness_tools::skills::PackRegistryHandle` is the late-bound, non-owning (`Weak`) view the
  proxy dispatches through. It holds two registries: the durable tool `Arc`
  and the agent's `synthesized_tools` `Arc`, where every `delegate_*` tool
  lives. Each must be rebound after its `Arc` is replaced, or packed
  tools degrade to "skill unavailable" / "no tool in skill". The handle rides
  on `Tool::host_extension` and is read back by
  `tools::host_extensions::pack_registry_handle`. The proxy itself
  (`UseSkillTool`, `PackCatalog`, `ToolPack`, the listing/scoping renderers) lives in
  `tinyagents_harness_tools::skills`; this module keeps the pack table
  (`registry::PACKS`, handed over as `registry::CATALOG`), the group posture and the
  registry binding.
- `ops::strip_packed_from_visible` does the actual compression: it removes a
  pack's tool names from an agent's advertised `visible` set (only those whose
  group is `Withheld`) and adds `use_skill` back in only if something was
  actually withheld. An empty `visible` set is the harness's "everything
  visible" sentinel and is left alone. `ops::is_withheld_from` exposes the
  same predicate for a caller building a tool *listing* rather than a
  `visible` set; nothing in-tree calls it today.

**Why a proxy instead of dynamic registration.** Registering the real schemas
mid-turn would be better (native tool calling, no nested `args` object), but
`tinyagents` registers tools into a plain `HashMap` behind `&mut` before the
turn starts and derives the provider-facing schema list once from that.
Making the registry interior-mutable and re-deriving schemas per iteration is
the upstream change that would retire this proxy.

## `ToolGroups`

`groups::ToolGroups` (fixed-size array, `GROUP_COUNT = PACKS.len()`) is the
`CoreBuilder`-facing narrowing control referenced from `AGENTS.md` ("tool
visibility with `ToolGroups`"). Per pack it holds a `groups::GroupMode`:

| Mode | Schemas on the wire | Registered and callable |
| --- | --- | --- |
| `Advertised` | yes | yes |
| `Withheld` (default) | no, reached via `use_skill` | yes |
| `Off` | no | no |

`ToolGroups::default()` / `packed()` puts every pack in `Withheld`, exactly
the compiled-in behavior before this type existed, so a host that never calls
`CoreBuilder::tool_groups` is unaffected. `advertised()`, `none()`, and
`with(id, mode)` are the other constructors; an unknown id in `with` is
logged and ignored, and a tool that belongs to no pack always reports
`Advertised`. This axis only narrows: a pack compiled out by a Cargo feature,
or off under the ambient `DomainSet`, stays absent regardless of the mode set
here. `groups::current()` reads the ambient `ToolGroups` off `CoreContext`,
falling back to `Withheld`-everywhere when there is no context (unit tests,
pre-boot CLI paths).

## Inline skill guides

A pack can carry a `guide`: a short playbook in `guides/<id>.md`,
`include_str!`'d into `registry::PACKS`, that `use_skill` prints between the
pack summary and the tool schemas when the pack is loaded. Packs with a guide
today: `coding` (search, edit, run scripts, test, lint, review a diff, git),
`web3` (wallet, swaps, bridges, contract calls, x402), `system` (config,
health, diagnostics, costs, service lifecycle, credentials, app updates),
`scheduling` (`cron`), `docs` (`gitbooks_search` / `gitbooks_get_page`) and
`mcp` (MCP registry search, connect, status, tool calls). The other packs are
plain schema bundles with an empty guide.

Why: these guides replaced the single-belt specialists (`code_executor`,
`crypto_agent`, `settings_agent`, `scheduler_agent`, `help`, `mcp_agent`,
`tool_maker`, `critic` as a chat reviewer, and similar). A sub-agent whose
whole value was a system prompt over a handful of tools cost a separate
context, a model call and a hand-off envelope on every use. As a guide it is
roughly 500 tokens, paid once when the skill is loaded, and the orchestrator
calls the tools itself. Those tools are `ToolExposure::Deferred` rather than
on any belt, so `tool_search` finds a single one of them without loading the
whole skill.

What does not go in a guide: a rule that has to bind before the model thinks
to load the skill. The two such rules (an explicit yes before moving funds or
executing a swap, bridge or contract call, and an explicit yes before
stopping, shutting down, uninstalling or updating OpenHuman's service) live
in the orchestrator prompt's `## Skills` section
(`agent_registry/agents/orchestrator/prompt.md`). Lookup detail and the step
by step loop belong in the guide.

[`toolpacks_tests.rs`](./toolpacks_tests.rs) keeps guides honest: every guided pack must carry a
non-empty guide, every guide stays under `GUIDE_BUDGET_BYTES` (2,400 bytes,
about 550 tokens), no guide may name a removed specialist's hand-off tool
(`run_code`, `do_crypto`, `manage_settings`, ...), every tool in a guided
pack must be named in its guide, and a loaded skill renders its guide before
its schemas.

Running an installed skill is not a pack concern: the orchestrator uses its
own `run_workflow`. The install hand-off `setup_skills` (into `skill_setup`)
is a member of the `skills` pack, like the other packed hand-offs
(`build_workflow`, `manage_tasks`): it was ~270 tokens on every orchestrator
request for a family used a few times a week. Being packed, it no longer
closes the `skills` pack through `ops::closed_by_direct_handoff`, so the
listing offers the hand-off beside the raw registry tools.

## Called by

- `tools/ops.rs`: `all_tools_with_runtime` drops every tool whose group is
  `Off`, then appends `use_skill` via `append_pack_tools`.
- `tools/host_extensions.rs`: `pack_registry_handle` reads the handle back off a
  tool's `host_extension`.
- `core/runtime/builder.rs` and `core/runtime/context.rs`: `CoreBuilder::tool_groups` and the ambient `ToolGroups` on `CoreContext`.
- `agent/session_host/builder/`: `builder_build.rs` strips packed names
  from the agent's visible set and binds both registries once the tool `Arc`s
  exist; [`mod.rs`](./mod.rs) calls `scope_use_skill_spec` so the advertised `use_skill`
  spec lists only packs this session can call something in, and drops the
  spec entirely when that is none.
- `integrations/prelude_integrations.rs`: rebinds the synthesized registry
  after every delegation refresh and re-strips packed names;
  `agent/session_host/runtime/accessors.rs` re-strips after
  materializing the visible set.
- `agent/tinyagents/middleware/tool_policy.rs`: after
  the permission gates, intercepts the disclosure half of a `use_skill` call
  (`named_tool` is `None`) and renders it with `render_pack_filtered` against
  the session's allowlist, with `route_sentence` naming the owner delegate
  when nothing in the pack is callable.
- `agent/tinyagents/middleware/packed_tool_route.rs`: `before_tool`
  rewrites a call that names a withheld packed tool by its bare name (the name
  the listing and sibling descriptions use) into the `use_skill` call that
  reaches it, ahead of admission, so the rewritten call passes every gate an
  explicit `use_skill` call does (#6276). It routes only when the turn's
  tool-policy session lets that tool run (`blocks_execution()` is false), and
  never on a turn without a session (sub-agent, channel/CLI), where an
  unregistered name was excluded by the registration allowlist.
- `agent_registry/agents/orchestrator/prompt.rs`: `pack_for_tool` to tell
  the orchestrator which pack a withheld delegate lives in.
- `crates/openhuman-embed/`: re-exports `GroupMode` and `ToolGroups` and
  exposes `Harness::builder().tool_groups(..)`.

## Tests

[`toolpacks_tests.rs`](./toolpacks_tests.rs) (pack-table invariants, owner rules,
`strip_packed_from_visible`, `use_skill` dispatch and permission / timeout /
effect forwarding, dual-registry binding) and
[`toolpacks_tests_scoping_and_visibility_tests.rs`](./toolpacks_tests_scoping_and_visibility_tests.rs)
(`scope_use_skill_spec`, `render_pack_filtered` filtering, `route_sentence`,
rebinding); [`groups_tests.rs`](./groups_tests.rs) (`GroupMode` defaults and narrowing rules).

## Further reading

- [Parent module (`tools`)](../README.md)
- [Native tools overview](../../../../../docs/gitbooks/en/features/native-tools/README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [tinyagents submodule](../../../../../vendor/tinyagents/README.md)
