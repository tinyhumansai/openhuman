# prompts

Owns the bundled prompt assets and the system-prompt rendering pipeline: the
types every section renders from, the section implementations themselves, the
builder that assembles them in tier order, and the free `render_*` helpers other
domains use to compose their own prompts.

## Public surface (from `mod.rs`)

- `types::*` — `PromptContext`, `PromptSection` trait, `PromptTier`,
  `PromptTool`, `ToolCallFormat`, `LearnedContextData`, `UserIdentity`,
  `ConnectedIntegration`, `NamespaceSummary`, `SubagentRenderOptions`. Pure
  data, no rendering logic. Also holds the `pub(crate)` caps
  `BOOTSTRAP_MAX_CHARS` (20K, for `SOUL.md`/`IDENTITY.md`/`ROLE.md` and each
  `AGENTS.md` layer) and `USER_FILE_MAX_CHARS` (2K, for `PROFILE.md`,
  `MEMORY.md` and the snapshot `USER.md` block).
- `render_connected_identities` (`connected_identities.rs`) — best-effort,
  sync render of the `## Connected Identities` block; reads through the bound
  memory driver via `block_in_place`, returns empty when there is no
  multi-thread runtime or the config/driver read fails.
- `agents_md::{load_agents_md, load_agents_md_layers, AgentsMdContent, AGENTS_MD_FILENAME}`
  — loads `AGENTS.md` at the global (`workspace_dir`) and local
  (`action_dir`, or a sub-agent's `worktree_action_dir` override) layers.
- `builder::{SystemPromptBuilder, TieredPrompt, GLOBAL_STYLE_SUFFIX}` —
  assembles ordered `PromptSection`s into a final prompt string (or a
  `TieredPrompt` with cache-breakpoint offsets).
- `sections::*` — the concrete `PromptSection` structs (`IdentitySection`,
  `ToolsSection`, `SafetySection`, `UserFilesSection`, `UserMemorySection`,
  `UserReflectionsSection`, `UserIdentitySection`, `WorkspaceSection`,
  `DateTimeSection`, `RuntimeSection`, `AgentsInstructionsSection`,
  `PersonalityRosterSection`, `ArchetypePromptSection`,
  `DynamicPromptSection`, `GroundingSection`).
- Named re-exports from `render_helpers` (`render_helpers.rs` plus the
  `render_helpers/` submodules `section_renderers.rs`, `subagent.rs`,
  `workspace_files.rs`) — free `render_*` functions
  (thin wrappers over the section structs), the workspace-file helpers
  (`inject_workspace_file[_capped]`, `inject_inline_content`,
  `inject_snapshot_content`, `sync_workspace_file`,
  `default_workspace_file_content`), `current_datetime_line`,
  `memory_date_label`, and the sub-agent renderer
  (`render_subagent_system_prompt[_with_format]`). Lets a caller assemble a
  prompt by calling functions directly instead of going through
  `SystemPromptBuilder`.

## Bundled assets

`IDENTITY.md`, `ROLE.md`, `SOUL.md`, `STYLE.md` are the bundled copies of the
master agent's identity, role brief, personality, and writing style. They are
embedded with `include_str!` in `render_helpers/workspace_files.rs`
(`default_workspace_file_content`) and, for `STYLE.md`, again in `builder.rs`
as `GLOBAL_STYLE_SUFFIX`. Two uses:

- **Seed and refresh** — `sync_workspace_file(workspace_dir, filename)` writes
  the bundled text to `<workspace_dir>/<filename>` when it is missing and
  records a hash of the bundled content in a `.<filename>.builtin-hash`
  sidecar. On later runs the file is left alone unless the bundled default
  changed (hash mismatch), in which case the disk copy is overwritten. So a
  user edit survives until the next release that touches that asset. Only
  absolute `workspace_dir`s are seeded. `IdentitySection` syncs
  `SOUL.md`/`IDENTITY.md`/`ROLE.md` (`ROLE.md` is injected only when
  `visible_tool_names` is non-empty, i.e. for the orchestrator); `builder.rs`
  syncs `STYLE.md` on every build so agents that set `omit_identity` still get
  the style rules.
- **Compile-time fallback** — `GLOBAL_STYLE_SUFFIX` is used when the
  workspace `STYLE.md` cannot be read.

`USER.md` is different: nothing in this module reads it. It is exposed as the
`openhuman://prompts/user` MCP resource by `mcp/server/resources.rs`
(`include_str!("../../agent/prompts/USER.md")`). The `USER.md` block that
appears in prompts comes from the curated-memory snapshot
(`CuratedMemoryPromptSnapshot.user`), not from this file.

Editing these files changes the shipped default agent's persona/style without
a code change.

## Canonical module

`agent::prompts` is the canonical prompt-plumbing module. Prompt logic lives
here so it sits next to the agents that consume it; do not add a forwarding
module for this API.

## Extension points (owned elsewhere)

Other domains contribute prompt content without living in this directory:

- `agent/learning/prompt_sections.rs` — `LearnedContextSection`,
  `UserProfileSection`, `MemoryAccessSection`, `MemoryWriteSection`
  (`PromptSection` impls, config-gated; `agent/session_host/builder/factory.rs`
  and `.../builder/helpers.rs` append them with `add_section` /
  `insert_section_before` when learning or explicit preferences are enabled).
- `tools/agent_policy/prompt.rs` — `render_tool_policy_boundary` is not a
  section: `agent/session_host/turn/context.rs` string-appends its
  `## Tool Policy Boundary` block after the builder output so the
  session-scoped bytes land at the tail of the prompt.

Built-in archetype system prompts (orchestrator, welcome, integrations_agent,
…) live in `agent/registry/agents/<name>/prompt.rs` and
`flows/agents/{flow_discovery,workflow_builder}/prompt.rs`, not here. Each is a
`PromptSource::Dynamic` function that hand-assembles its body via the
`render_*` helpers; `agent/session_host/builder/factory.rs` wraps it with
`SystemPromptBuilder::from_dynamic`.

## Builder entry points

- `SystemPromptBuilder::with_defaults()` — the primary-agent chain (identity,
  user files, `AGENTS.md`, user memory, tools, safety, workspace, datetime,
  runtime).
- `SystemPromptBuilder::for_subagent(archetype_prompt_text, omit_identity,
  omit_safety_preamble)` — narrow chain driven by a sub-agent definition's
  `omit_*` flags; deliberately excludes `DateTimeSection` so repeat spawns of
  the same definition stay byte-identical for prefix-cache reuse.
- `SystemPromptBuilder::from_dynamic(builder)` — wraps a
  `PromptSource::Dynamic` fn pointer in a `DynamicPromptSection` and still
  injects the shared `AgentsInstructionsSection`. `from_final_body(body)`
  wraps an already-rendered string the same way; it currently has only test
  callers.

Every `build()` appends the grounding contract (unless the body already
carries the `GROUNDING_HEADING`) and the workspace `STYLE.md` block, so all
three chains share the same anti-fabrication floor and style rules.

## KV-cache / prefix stability

The rendered prompt is built once per session and reused on every turn
(`agent/session_host/turn/context.rs`) so the inference backend's prefix
cache hits. `PromptSection::tier()` (`PromptTier::Stable` / `Context` /
`Volatile`, default `Stable`) controls emission order in
`SystemPromptBuilder::build_tiered`: stable bytes (identity, tools, safety,
datetime rules) first, then per-session context (`AGENTS.md`, workspace,
runtime), then volatile bytes (user files, memory, reflections, signed-in
identity, personality roster, the dynamic archetype body) last, with a
breakpoint offset recorded after each non-empty tier. A prefix is reusable
only up to the first differing byte, so a volatile section rendered early
invalidates every stable byte behind it. Two consequences visible in this
module: `DateTimeSection` renders only the clock *rules* and is `Stable` (the
live timestamp rides the user message via `current_datetime_line`), and
`memory_date_label` renders `NamespaceSummary.updated_at` as an absolute date
rather than "N days ago".

## Used by

- `agent/session_host/turn/context.rs` — loads `AGENTS.md` layers and
  connected identities into `PromptContext`, calls the builder, appends the
  tool-policy boundary.
- `agent/session_host/builder/factory.rs` — picks the entry point per
  `PromptSource` and registers the learning/profile sections.
- `agent/debug/` — `dump_agent_prompt` / `dump_all_agent_prompts` (`mod.rs`)
  build the same `PromptContext` to render each agent's prompt,
  `dump_writer.rs` writes it to disk, `prompt_size.rs` measures it.
