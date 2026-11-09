# Agent prompt adapters

`agent::prompts` owns the bundled identity assets and OpenHuman-specific
adapters used by agent prompt composition. The portable prompt section API,
tier assembly, and dialect rendering belong to TinyAgents and TinyTools.
OpenHuman selects product content and preserves host policy and provider
message boundaries.

## Public surface

- `types::*`: `PromptContext`, `PromptTool`, `ToolCallFormat`, `UserIdentity`,
  `ConnectedIntegration`, `SubagentRenderOptions`, and the 20K
  `BOOTSTRAP_MAX_CHARS` content cap. Prompt tiers and marker parsing are
  re-exported from TinyAgents for source compatibility.
- `agents_md::*`: thin adapters around TinyAgents' layered `AGENTS.md` loader.
  The adapter applies OpenHuman's path-root policy; the sibling tests cover
  symlink and FIFO rejection, bounded reads, and layer selection.
- `SystemPromptBuilder`, `TieredPrompt`, and `GLOBAL_STYLE_SUFFIX`:
  compatibility exports of the product selector in
  [`crate::prompts::composition`](../../prompts/composition.rs). It selects
  OpenHuman content, appends the grounding and workspace style rules, and
  delegates prompt tier composition to TinyAgents' `PromptComposer`. Stable and
  Context parts remain merged into the first system message; Volatile parts
  remain in a second message.
- `sections::*`: OpenHuman-owned prompt content renderers. The portable section
  contract and tier assembly are implemented by the owner crate.
- `render_subagent_system_prompt*`: OpenHuman's narrow subagent prompt adapter.
  TinyTools renders the protocol for its selected tool-call dialect.
- Other named exports include workspace file sync/injection helpers,
  connected identities, and renderer helpers used by product agents.

## Bundled assets

`IDENTITY.md`, `ROLE.md`, `SOUL.md`, and [`STYLE.md`](./STYLE.md) are the
bundled prompt assets. `sync_workspace_file` seeds a workspace copy, refreshes
an unmodified copy when bundled content changes, and preserves user edits.
The product prompt
composition reads the workspace `STYLE.md`, falling back to the bundled copy
when the file cannot be read. [`USER.md`](./USER.md) is exposed as an MCP
resource and is not read by this module.

Built-in archetype prompts generally live in
`agent_registry/agents/<name>/prompt.rs`; flow prompts live with their flow
agents. `PromptSource::Dynamic` functions provide product content, and the
OpenHuman adapter adds the shared AGENTS.md, grounding, and style content.

[`USER.md`](./USER.md) is different: nothing in this module reads it. It is exposed as the
`openhuman://prompts/user` MCP resource by [`mcp/server/resources.rs`](../../mcp/server/resources.rs)
(`include_str!("../../agent/prompts/USER.md")`).

The user's memory is not part of the system prompt: memory v2 injects the
compiled `context.md` as the first user message of a new session (gated by
`AgentDefinition::omit_memory_context`), and agents reach anything deeper
through the `memory` tool.

Editing these files changes the shipped default agent's persona and style
without a code change.

## Canonical module

`agent::prompts` is the canonical prompt-plumbing module. Prompt logic lives
here so it sits next to the agents that consume it; do not add a forwarding
module for this API.

## Extension points (owned elsewhere)

Other domains contribute prompt content without living in this directory:

- [`tools/agent_policy/prompt.rs`](../../tools/agent_policy/prompt.rs): `render_tool_policy_boundary` is not a
  section. [`agent/session_host/turn/context.rs`](../session_host/turn/context.rs) string-appends its
  `## Tool Policy Boundary` block after the builder output so the
  session-scoped bytes land at the tail of the prompt.

Built-in archetype system prompts (orchestrator, planner, image_agent,
and so on), including the flow builtins, live in
`agent/registry/agents/<name>/prompt.rs`, not here. Each is a
`PromptSource::Dynamic` function that hand-assembles its body via the
`render_*` helpers; [`agent/session_host/builder/factory.rs`](../session_host/builder/factory.rs) wraps it with
`SystemPromptBuilder::from_dynamic`.

## Builder entry points

- `SystemPromptBuilder::with_defaults()`: the primary-agent chain (identity,
  `AGENTS.md`, tools, safety, workspace, datetime, runtime).
- `SystemPromptBuilder::for_subagent(archetype_prompt_text, omit_identity,
  omit_safety_preamble)`: narrow chain driven by a sub-agent definition's
  `omit_*` flags. It deliberately excludes `DateTimeSection` so repeat spawns
  of the same definition stay byte-identical for prefix-cache reuse.
- `SystemPromptBuilder::from_dynamic(builder)`: wraps a
  `PromptSource::Dynamic` fn pointer in a `DynamicPromptSection` and still
  injects the shared `AgentsInstructionsSection`. `from_final_body(body)`
  wraps an already-rendered string the same way; it currently has only test
  callers.

Every `build()` appends the grounding contract (unless the body already
carries the `GROUNDING_HEADING`) and the workspace `STYLE.md` block, so all
three chains share the same anti-fabrication floor and style rules.

## KV-cache and prefix stability

The rendered prompt is built once per session and reused on every turn
(`agent/session_host/runtime_session.rs::prepare`) so the inference backend's
prefix cache hits. `PromptSection::tier()` (`PromptTier::Stable` / `Context` /
`Volatile`, default `Stable`) controls emission order in
`SystemPromptBuilder::build_tiered`: every section renders once through
`PromptSection::build_parts` and its parts are bucketed by tier. Stable bytes
(identity, rules, tool protocol, datetime rules, the shared grounding contract
and `STYLE.md`) come first, then per-session context (`AGENTS.md`, workspace,
model-gated execution discipline), then volatile bytes (the signed-in user,
installed skills, connected integrations and MCP servers) last.

A `PromptSource::Dynamic` builder declares its own tiers by emitting
`PROMPT_TIER_CONTEXT_MARKER` / `PROMPT_TIER_VOLATILE_MARKER` on their own lines
(`split_prompt_tiers`); a builder that emits neither stays wholly `Volatile`.
The orchestrator does this so its identity and rules lead the stable tier
instead of trailing the volatile sections.

`TieredPrompt::system_messages()` hands the session one system message for
`Stable + Context` and a second for `Volatile`. The tinyagents harness gives
each leading system message its own cacheable segment
(`PromptBuilder::push_system_messages`), so a newly
connected service changes the second segment and leaves the first
byte-identical (`system`, `system.1`). A prefix is reusable only up to the first differing
byte, so a volatile section rendered early invalidates every stable byte
behind it. One consequence is visible in this module: `DateTimeSection`
renders only the clock *rules* and is `Stable` (the live timestamp rides the
user message via `current_datetime_line`).

## Used by

- `agent/session_host/turn/context.rs`: loads `AGENTS.md` layers and
  connected identities into `PromptContext`, calls the builder, and appends
  the tool-policy boundary.
- `agent/session_host/builder/factory.rs`: picks the entry point per
  `PromptSource`.
- [`agent/debug/`](../debug/): `dump_agent_prompt` / `dump_all_agent_prompts` ([`mod.rs`](./mod.rs))
  build the same `PromptContext` to render each agent's prompt,
  `dump_writer.rs` writes it to disk, `prompt_size.rs` measures it.

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Personalization and self-learning](../../../../../docs/gitbooks/en/features/personalization.md)
The session host builds a prompt once for a session. Stable and Context
content forms the first system message, and Volatile content forms the second.
Resumed sessions reuse persisted system messages verbatim, preserving their
provider cache prefix.
