# prompts

Owns the bundled prompt assets and the system-prompt rendering pipeline: the
types every section renders from, the section implementations themselves, the
builder that assembles them in tier order, and the free `render_*` helpers other
domains use to compose their own prompts.

## Public surface (from `mod.rs`)

- `types::*`: `PromptContext`, `PromptSection` trait, `PromptTier`,
  `PromptTool`, `ToolCallFormat`, `UserIdentity`, `ConnectedIntegration`,
  `SubagentRenderOptions`. Pure data, no rendering logic. Also holds the
  `pub(crate)` cap `BOOTSTRAP_MAX_CHARS` (20K, for
  `SOUL.md`/`IDENTITY.md`/`ROLE.md` and each `AGENTS.md` layer).
- `render_connected_identities` (`connected_identities.rs`): best-effort,
  sync render of the `## Connected Identities` block. It reads through the
  bound memory driver via `block_in_place` and returns empty when there is no
  multi-thread runtime or the config/driver read fails.
- `agents_md::{load_agents_md, load_agents_md_layers, AgentsMdContent, AGENTS_MD_FILENAME}`:
  loads `AGENTS.md` at the global (`workspace_dir`) and local
  (`action_dir`, or a sub-agent's `worktree_action_dir` override) layers.
- `builder::{SystemPromptBuilder, TieredPrompt, GLOBAL_STYLE_SUFFIX}`:
  assembles ordered `PromptSection`s into a final prompt string (or a
  `TieredPrompt` with cache-breakpoint offsets).
- `sections::*`: the concrete `PromptSection` structs (`IdentitySection`,
  `ToolsSection`, `SafetySection`, `UserIdentitySection`, `WorkspaceSection`,
  `DateTimeSection`, `RuntimeSection`, `AgentsInstructionsSection`,
  `PersonalityRosterSection`, `ArchetypePromptSection`,
  `DynamicPromptSection`, `GroundingSection`).
- Named re-exports from `render_helpers` (`render_helpers.rs` plus the
  `render_helpers/` submodules `section_renderers.rs`, `subagent.rs`,
  `workspace_files.rs`): free `render_*` functions (thin wrappers over the
  section structs), the workspace-file helpers
  (`inject_workspace_file[_capped]`, `inject_inline_content`,
  `sync_workspace_file`, `default_workspace_file_content`),
  `current_datetime_line`, and the sub-agent renderer
  (`render_subagent_system_prompt[_with_format]`). These let a caller
  assemble a prompt by calling functions directly instead of going through
  `SystemPromptBuilder`.

## Bundled assets

`IDENTITY.md`, `ROLE.md`, `SOUL.md`, `STYLE.md` are the bundled copies of the
master agent's identity, role brief, personality, and writing style. They are
embedded with `include_str!` in `render_helpers/workspace_files.rs`
(`default_workspace_file_content`) and, for `STYLE.md`, again in `builder.rs`
as `GLOBAL_STYLE_SUFFIX`. There are two uses:

- Seed and refresh: `sync_workspace_file(workspace_dir, filename)` writes the
  bundled text to `<workspace_dir>/<filename>` when it is missing and records
  a hash of the bundled content in a `.<filename>.builtin-hash` sidecar. On
  later runs the file is left alone unless the bundled default changed (hash
  mismatch), in which case the disk copy is overwritten, so a user edit
  survives until the next release that touches that asset. Only absolute
  `workspace_dir`s are seeded. `IdentitySection` syncs
  `SOUL.md`/`IDENTITY.md`/`ROLE.md` (`ROLE.md` is injected only when
  `visible_tool_names` is non-empty, i.e. for the orchestrator); `builder.rs`
  syncs `STYLE.md` on every build so agents that set `omit_identity` still get
  the style rules.
- Compile-time fallback: `GLOBAL_STYLE_SUFFIX` is used when the workspace
  `STYLE.md` cannot be read.

`USER.md` is different: nothing in this module reads it. It is exposed as the
`openhuman://prompts/user` MCP resource by `mcp/server/resources.rs`
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

- `tools/agent_policy/prompt.rs`: `render_tool_policy_boundary` is not a
  section. `agent/session_host/turn/context.rs` string-appends its
  `## Tool Policy Boundary` block after the builder output so the
  session-scoped bytes land at the tail of the prompt.

Built-in archetype system prompts (orchestrator, planner, image_agent,
and so on) live in `agent/registry/agents/<name>/prompt.rs` and
`flows/agents/{flow_discovery,workflow_builder}/prompt.rs`, not here. Each is a
`PromptSource::Dynamic` function that hand-assembles its body via the
`render_*` helpers; `agent/session_host/builder/factory.rs` wraps it with
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
- `agent/debug/`: `dump_agent_prompt` / `dump_all_agent_prompts` (`mod.rs`)
  build the same `PromptContext` to render each agent's prompt,
  `dump_writer.rs` writes it to disk, `prompt_size.rs` measures it.
