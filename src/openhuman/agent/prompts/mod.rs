pub mod types;
pub use types::*;

use crate::openhuman::skills::Skill;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use chrono::Local;
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Default)]
pub struct SystemPromptBuilder {
    sections: Vec<Box<dyn PromptSection>>,
}

impl SystemPromptBuilder {
    pub fn with_defaults() -> Self {
        Self {
            sections: vec![
                Box::new(IdentitySection),
                // User files (PROFILE.md, MEMORY.md) ride right after the
                // identity bootstrap so they land in the cache-friendly
                // prefix alongside SOUL/IDENTITY. Gated per-agent — see
                // `UserFilesSection`. Intentionally separate from
                // `IdentitySection` so agents that strip the identity
                // preamble via `for_subagent(omit_identity=true)` still
                // get their user files (welcome / orchestrator / the
                // trigger pair).
                Box::new(UserFilesSection),
                // User memory sits right after the identity bootstrap so the
                // model has rich, persistent context about the user before it
                // sees the tool catalogue. Section is empty (and skipped) when
                // the tree summarizer has nothing on disk yet.
                Box::new(UserMemorySection),
                Box::new(ToolsSection),
                Box::new(SafetySection),
                Box::new(WorkspaceSection),
                Box::new(DateTimeSection),
                Box::new(RuntimeSection),
            ],
        }
    }

    /// Build a narrow prompt for a sub-agent.
    ///
    /// The sub-agent's archetype prompt is registered as a dedicated
    /// section that always renders first. The remaining sections respect
    /// the `omit_*` flags from the [`crate::openhuman::agent::harness::definition::AgentDefinition`]:
    /// `omit_identity` skips the project-context dump, `omit_safety_preamble`
    /// skips the safety rules, and so on. The `WorkspaceSection` is always
    /// included so the sub-agent knows its working directory.
    ///
    /// `archetype_prompt_text` is the already-loaded body of the
    /// `system_prompt` source on the definition (the runner resolves
    /// inline vs file before calling this).
    ///
    /// # KV cache stability
    ///
    /// `DateTimeSection` is intentionally **not** included here.
    /// Repeat spawns of the same sub-agent definition must produce
    /// byte-identical system prompts so the inference backend's
    /// automatic prefix cache can reuse the prefill from the previous
    /// run. Injecting `Local::now()` into the prompt would defeat that
    /// goal — if a sub-agent genuinely needs the current time it
    /// should receive it via the user message, not the system prompt.
    pub fn for_subagent(
        archetype_prompt_text: String,
        omit_identity: bool,
        omit_safety_preamble: bool,
        _omit_skills_catalog: bool,
    ) -> Self {
        let mut sections: Vec<Box<dyn PromptSection>> =
            vec![Box::new(ArchetypePromptSection::new(archetype_prompt_text))];

        if !omit_identity {
            sections.push(Box::new(IdentitySection));
        }
        // User files (PROFILE.md / MEMORY.md) are gated independently of
        // `omit_identity` so agents that drop the identity preamble (e.g.
        // welcome's `omit_identity = true`) still surface the user's
        // onboarding + archivist context when `omit_profile` /
        // `omit_memory_md` are opted in.
        sections.push(Box::new(UserFilesSection));
        // Tools section is always included — the sub-agent needs to see
        // its own (filtered) tool catalogue.
        sections.push(Box::new(ToolsSection));
        if !omit_safety_preamble {
            sections.push(Box::new(SafetySection));
        }
        // Skills catalogue and connected integrations are rendered by
        // the individual agent's `prompt.rs` when that agent needs
        // them (skills_agent for the skill-executor voice,
        // orchestrator/welcome for the delegator voice). The shared
        // builder intentionally does not emit them — keeping
        // agent-specific prose scoped to the agent that owns it.
        sections.push(Box::new(WorkspaceSection));

        Self { sections }
    }

    /// Build from a fully-assembled prompt string — no section wrapping.
    ///
    /// Used when the caller has already composed the final prompt (e.g.
    /// via a function-driven `PromptSource::Dynamic` builder that calls
    /// the `render_*` section helpers itself). The returned builder has
    /// a single [`ArchetypePromptSection`] containing the body verbatim.
    pub fn from_final_body(body: String) -> Self {
        Self {
            sections: vec![Box::new(ArchetypePromptSection::new(body))],
        }
    }

    pub fn add_section(mut self, section: Box<dyn PromptSection>) -> Self {
        self.sections.push(section);
        self
    }

    /// Render every section in order into a single prompt string.
    ///
    /// The rendered bytes are intended to be **frozen for the whole
    /// session** — callers build the system prompt once at session
    /// start and reuse the exact bytes on every subsequent turn so the
    /// inference backend's prefix cache hits uniformly. There is no
    /// cache-boundary marker to emit because the entire prompt is
    /// static from the provider's perspective.
    pub fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let mut output = String::new();
        for section in &self.sections {
            let part = section.build(ctx)?;
            if part.trim().is_empty() {
                continue;
            }
            output.push_str(part.trim_end());
            output.push_str("\n\n");
        }
        Ok(output)
    }
}

/// Sub-agent role prompt — pre-loaded text from an
/// [`crate::openhuman::agent::harness::definition::AgentDefinition`]'s
/// `system_prompt` field. Always rendered first when present.
pub struct ArchetypePromptSection {
    body: String,
}

impl ArchetypePromptSection {
    pub fn new(body: String) -> Self {
        Self { body }
    }
}

impl PromptSection for ArchetypePromptSection {
    fn name(&self) -> &str {
        "archetype_prompt"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        if self.body.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(self.body.clone())
    }
}

pub struct IdentitySection;
pub struct ToolsSection;
pub struct SafetySection;
// `SkillsSection` and `ConnectedIntegrationsSection` previously lived
// here and branched on `ctx.agent_id` to pick between the skill-
// executor and delegator voice. They've been removed — each agent's
// `prompt.rs` now renders its own block inline (skills_agent owns the
// `## Available Skills` + executor-voice `## Connected Integrations`
// blocks, orchestrator owns `## Delegation Guide — Integrations`,
// welcome owns its onboarding-flavoured connected list).
pub struct WorkspaceSection;
pub struct RuntimeSection;
pub struct DateTimeSection;
pub struct UserMemorySection;

/// Injects the user-specific, session-frozen workspace files
/// (`PROFILE.md` + `MEMORY.md`), each capped at [`USER_FILE_MAX_CHARS`].
///
/// Separate from [`IdentitySection`] so agents that strip the project-
/// context preamble (`omit_identity = true` — welcome, orchestrator,
/// the trigger pair) still get their user-file injection at runtime via
/// [`SystemPromptBuilder::for_subagent`], which skips `IdentitySection`
/// entirely when `omit_identity` is on.
///
/// Cache-stability: static per session — the whole point of the
/// 2000-char cap and the load-once rule documented on
/// [`AgentDefinition::omit_profile`] / `omit_memory_md`.
pub struct UserFilesSection;

impl PromptSection for IdentitySection {
    fn name(&self) -> &str {
        "identity"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let mut prompt = String::from("## Project Context\n\n");
        prompt.push_str(
            "The following workspace files define your identity, behavior, and context.\n\n",
        );
        // When the visible-tool filter is active the main agent is a pure
        // orchestrator: it routes via spawn_subagent, synthesises results,
        // and talks to the user. It does NOT need the periodic-task config
        // (HEARTBEAT.md) — subagents handle their own concerns.
        let is_orchestrator = !ctx.visible_tool_names.is_empty();
        let all_files: &[&str] = &["SOUL.md", "IDENTITY.md", "HEARTBEAT.md"];
        // Orchestrator skips these from the prompt but we still sync them
        // to disk so they stay current.
        let skip_in_prompt: &[&str] = if is_orchestrator {
            &["HEARTBEAT.md"]
        } else {
            &[]
        };
        for file in all_files {
            // Always sync to disk so builtin updates ship.
            sync_workspace_file(ctx.workspace_dir, file);
            if !skip_in_prompt.contains(file) {
                inject_workspace_file(&mut prompt, ctx.workspace_dir, file);
            }
        }

        // PROFILE.md / MEMORY.md injection lives in the dedicated
        // `UserFilesSection` (below) so agents that strip the identity
        // preamble (`omit_identity = true`) — welcome, orchestrator, the
        // trigger pair — still get their user files at runtime via
        // `SystemPromptBuilder::for_subagent`, which omits
        // `IdentitySection` entirely when `omit_identity` is set.

        Ok(prompt)
    }
}

impl PromptSection for UserFilesSection {
    fn name(&self) -> &str {
        "user_files"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        // Gate on the per-agent flags derived from
        // `AgentDefinition::omit_profile` / `omit_memory_md`. Both files
        // are user-specific, potentially growing, and capped at
        // [`USER_FILE_MAX_CHARS`] (~1000 tokens) so they can't bloat the
        // cached prefix.
        //
        // KV-cache contract: once injected into a session's rendered
        // prompt, the bytes are frozen for the remainder of that
        // session — any mid-session archivist write or enrichment
        // refresh lands on the NEXT session, never the in-flight one.
        let mut out = String::new();
        if ctx.include_profile {
            inject_workspace_file_capped(
                &mut out,
                ctx.workspace_dir,
                "PROFILE.md",
                USER_FILE_MAX_CHARS,
            );
        }
        if ctx.include_memory_md {
            inject_workspace_file_capped(
                &mut out,
                ctx.workspace_dir,
                "MEMORY.md",
                USER_FILE_MAX_CHARS,
            );
        }
        Ok(out)
    }
}

impl PromptSection for ToolsSection {
    fn name(&self) -> &str {
        "tools"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let mut out = String::from("## Tools\n\n");
        let has_filter = !ctx.visible_tool_names.is_empty();
        for tool in ctx.tools {
            // Skip tools not in the visible set when a filter is active.
            if has_filter && !ctx.visible_tool_names.contains(tool.name) {
                continue;
            }

            match ctx.tool_call_format {
                ToolCallFormat::PFormat => {
                    // P-Format renders a positional signature line:
                    // `**name[a|b]**: description`. The signature comes
                    // straight from the parameter schema (alphabetical
                    // by property name — see `pformat` module docs for
                    // why), so the model and the parser agree on
                    // argument ordering. We deliberately do NOT print
                    // the full JSON schema here: that's exactly the
                    // ~25-token-per-tool overhead p-format exists to
                    // eliminate.
                    let signature = render_pformat_signature_for_prompt(tool);
                    let _ = writeln!(
                        out,
                        "- **{}**: {}\n  Call as: `{}`",
                        tool.name, tool.description, signature
                    );
                }
                ToolCallFormat::Json | ToolCallFormat::Native => {
                    if let Some(schema) = &tool.parameters_schema {
                        let _ = writeln!(
                            out,
                            "- **{}**: {}\n  Parameters: `{}`",
                            tool.name, tool.description, schema
                        );
                    } else {
                        let _ = writeln!(out, "- **{}**: {}", tool.name, tool.description);
                    }
                }
            }
        }
        if !ctx.dispatcher_instructions.is_empty() {
            out.push('\n');
            out.push_str(ctx.dispatcher_instructions);
        }
        Ok(out)
    }
}

/// Build a P-Format signature line (`name[a|b|c]`) from a `&dyn Tool`.
/// Used by `render_subagent_system_prompt` which operates on `Box<dyn Tool>`
/// directly (no intermediate `PromptTool`). Mirrors the `PromptTool` variant
/// below — both BTreeMap-iterate the schema's `properties` in the same order.
fn render_pformat_signature_for_box_tool(tool: &dyn crate::openhuman::tools::Tool) -> String {
    let schema = tool.parameters_schema();
    let names: Vec<String> = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    if names.is_empty() {
        format!("{}[]", tool.name())
    } else {
        format!("{}[{}]", tool.name(), names.join("|"))
    }
}

/// Build a P-Format signature line (`name[a|b|c]`) from a [`PromptTool`].
/// Local to this module so [`ToolsSection`] doesn't have to depend on
/// the agent crate's `pformat` helper. The two implementations stay in
/// lockstep — both use BTreeMap iteration order on the schema's
/// `properties` field.
fn render_pformat_signature_for_prompt(tool: &PromptTool<'_>) -> String {
    let names: Vec<String> = tool
        .parameters_schema
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| {
            v.get("properties")
                .and_then(|p| p.as_object())
                .map(|m| m.keys().cloned().collect())
        })
        .unwrap_or_default();
    if names.is_empty() {
        format!("{}[]", tool.name)
    } else {
        format!("{}[{}]", tool.name, names.join("|"))
    }
}

impl PromptSection for SafetySection {
    fn name(&self) -> &str {
        "safety"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok("## Safety\n\n- Do not exfiltrate private data.\n- Do not run destructive commands without asking.\n- Do not bypass oversight or approval mechanisms.\n- Prefer `trash` over `rm`.\n- When in doubt, ask before acting externally.".into())
    }
}

impl PromptSection for WorkspaceSection {
    fn name(&self) -> &str {
        "workspace"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        Ok(format!(
            "## Workspace\n\nWorking directory: `{}`",
            ctx.workspace_dir.display()
        ))
    }
}

impl PromptSection for RuntimeSection {
    fn name(&self) -> &str {
        "runtime"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let host =
            hostname::get().map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().to_string());
        Ok(format!(
            "## Runtime\n\nHost: {host} | OS: {} | Model: {}",
            std::env::consts::OS,
            ctx.model_name
        ))
    }
}

impl PromptSection for UserMemorySection {
    fn name(&self) -> &str {
        "user_memory"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.tree_root_summaries.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Memory\n\n");
        out.push_str(
            "Long-term memory distilled by the tree summarizer. \
             Each section is the root summary for a memory namespace, \
             representing everything we've learned about that domain over time. \
             Treat this as durable context: the model has seen these facts before, \
             they should not need to be re-discovered.\n\n",
        );

        for (namespace, body) in &ctx.learned.tree_root_summaries {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                continue;
            }
            let _ = writeln!(out, "### {namespace}\n");
            out.push_str(trimmed);
            out.push_str("\n\n");
        }

        Ok(out)
    }
}

impl PromptSection for DateTimeSection {
    fn name(&self) -> &str {
        "datetime"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        let now = Local::now();
        Ok(format!(
            "## Current Date & Time\n\n{} ({})",
            now.format("%Y-%m-%d %H:%M:%S"),
            now.format("%Z")
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Section helpers for function-driven prompts
// ─────────────────────────────────────────────────────────────────────────────
//
// Each of the `Section` unit structs above is also available as a free
// `render_*` function that takes the same `PromptContext` and returns
// the section body (or an empty string when the section's gate is
// closed).
//
// These exist so `agents/<id>/prompt.rs` builders can assemble their own
// final system prompt, composing the exact sections they care about in
// the order they want — no `SystemPromptBuilder` machinery required.

/// Render the `## Project Context` identity block
/// (`SOUL.md` / `IDENTITY.md` / optionally `HEARTBEAT.md`).
pub fn render_identity(ctx: &PromptContext<'_>) -> Result<String> {
    IdentitySection.build(ctx)
}

/// Render the `PROFILE.md` + `MEMORY.md` user-file injection.
/// Empty when neither `ctx.include_profile` nor `ctx.include_memory_md`
/// is set.
pub fn render_user_files(ctx: &PromptContext<'_>) -> Result<String> {
    UserFilesSection.build(ctx)
}

/// Render the tree-summariser user-memory block.
pub fn render_user_memory(ctx: &PromptContext<'_>) -> Result<String> {
    UserMemorySection.build(ctx)
}

/// Render the `## Tools` catalogue in the dispatcher's tool-call format.
pub fn render_tools(ctx: &PromptContext<'_>) -> Result<String> {
    ToolsSection.build(ctx)
}

/// Render the static `## Safety` block.
pub fn render_safety() -> String {
    SafetySection
        .build(&empty_prompt_context_for_static_sections())
        .expect("SafetySection::build is infallible")
}

// `render_skills` and `render_connected_integrations` helpers are
// gone — `## Available Skills` lives in `skills_agent/prompt.rs`, and
// the connected-integrations / delegation-guide blocks each live in
// their owning agent's `prompt.rs` so no branching-on-agent-id logic
// needs to exist here.

/// Render the `## Workspace` block (working directory + file listing
/// bounds) — part of the dynamic, per-request suffix.
pub fn render_workspace(ctx: &PromptContext<'_>) -> Result<String> {
    WorkspaceSection.build(ctx)
}

/// Render the `## Runtime` block (model name, dispatcher format) —
/// dynamic.
pub fn render_runtime(ctx: &PromptContext<'_>) -> Result<String> {
    RuntimeSection.build(ctx)
}

/// Render the `## Current Date & Time` block. Intentionally **not**
/// included in byte-stable sub-agent prompts (`for_subagent`) because
/// injecting `Local::now()` defeats prefix caching. Exposed so full-
/// assembly main-agent builders can opt in.
pub fn render_datetime(ctx: &PromptContext<'_>) -> Result<String> {
    DateTimeSection.build(ctx)
}

/// Build a throwaway `PromptContext` for sections whose `build` only
/// uses static/immutable inputs (currently just `SafetySection`). Keeps
/// the `render_safety()` free function from forcing callers to
/// manufacture a full context when they only need the static text.
fn empty_prompt_context_for_static_sections() -> PromptContext<'static> {
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_SKILLS: &[Skill] = &[];
    static EMPTY_INTEGRATIONS: &[ConnectedIntegration] = &[];
    // SAFETY: the &HashSet reference must outlive the returned context;
    // a leaked OnceLock-style allocation gives us a permanent 'static
    // anchor without adding runtime cost on the hot path.
    static EMPTY_VISIBLE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(std::collections::HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new(""),
        model_name: "",
        agent_id: "",
        tools: EMPTY_TOOLS,
        skills: EMPTY_SKILLS,
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        include_profile: false,
        include_memory_md: false,
    }
}

/// Render a narrow, KV-cache-stable system prompt for a typed sub-agent.
///
/// This is a purpose-built alternative to
/// [`SystemPromptBuilder::for_subagent`] for call sites that only have
/// indices into the parent's `&[Box<dyn Tool>]` vec (so they can't
/// cheaply build a filtered owning slice for `ToolsSection`). The
/// output mirrors what `for_subagent` would emit with the matching
/// `omit_*` flags, plus a sub-agent-specific calling-convention
/// preamble and a model-only runtime banner.
///
/// `archetype_body` is the already-loaded archetype markdown — for
/// `PromptSource::Inline` this is the inline string, for
/// `PromptSource::File` this is the file contents loaded by the caller.
/// Callers resolve the source exactly once and hand the body in, so
/// this renderer works uniformly for both definition shapes.
///
/// `options` carries the per-definition rendering flags (safety, etc.)
/// inverted into positive-sense `include_*` form.
/// [`SubagentRenderOptions::narrow`] preserves the historical behaviour.
///
/// # KV cache stability
///
/// The rendered bytes MUST be a pure function of:
/// - the `archetype_body` (archetype role prompt)
/// - the filtered tool set (names, descriptions, schemas)
/// - the workspace directory
/// - the resolved model name
/// - the `options` (all static per definition)
///
/// Anything that varies across invocations at the *same* call site
/// (e.g. `chrono::Local::now()`, hostnames, pids, turn counters) is
/// forbidden here. Repeat spawns of the same sub-agent within a session
/// must produce byte-identical system prompts so the inference
/// backend's automatic prefix caching can reuse the prefill from the
/// previous run. Time-of-day information, if a sub-agent needs it,
/// belongs in the user message — not the system prompt.
pub fn render_subagent_system_prompt(
    workspace_dir: &Path,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn Tool>],
    extra_tools: &[Box<dyn Tool>],
    archetype_body: &str,
    options: SubagentRenderOptions,
    tool_call_format: ToolCallFormat,
    connected_integrations: &[ConnectedIntegration],
) -> String {
    render_subagent_system_prompt_with_format(
        workspace_dir,
        model_name,
        allowed_indices,
        parent_tools,
        extra_tools,
        archetype_body,
        options,
        tool_call_format,
        connected_integrations,
    )
}

/// Inner renderer that accepts an explicit [`ToolCallFormat`] so callers
/// that know the active dispatcher format can thread it through. The
/// public [`render_subagent_system_prompt`] defaults to PFormat for
/// backwards compatibility.
pub fn render_subagent_system_prompt_with_format(
    workspace_dir: &Path,
    model_name: &str,
    allowed_indices: &[usize],
    parent_tools: &[Box<dyn Tool>],
    extra_tools: &[Box<dyn Tool>],
    archetype_body: &str,
    options: SubagentRenderOptions,
    tool_call_format: ToolCallFormat,
    connected_integrations: &[ConnectedIntegration],
) -> String {
    let mut out = String::new();

    // 1. Archetype role prompt. Works for `PromptSource::Inline`,
    //    `PromptSource::File`, and `PromptSource::Dynamic` because the
    //    caller preloaded the body via `load_prompt_source`.
    let trimmed = archetype_body.trim();
    if !trimmed.is_empty() {
        out.push_str(trimmed);
        out.push_str("\n\n");
    }

    // 1b. Optional identity block. Off by default; turned on when the
    //     definition sets `omit_identity = false`. Renders the same
    //     OpenClaw bootstrap files the main agent loads, keeping the
    //     byte layout stable across repeat spawns of the same
    //     definition within a session.
    if options.include_identity {
        out.push_str("## Project Context\n\n");
        out.push_str(
            "The following workspace files define your identity, behavior, and context.\n\n",
        );
        for file in &["SOUL.md", "IDENTITY.md"] {
            inject_workspace_file(&mut out, workspace_dir, file);
        }
    }

    // 1c. PROFILE.md (onboarding enrichment output) and MEMORY.md
    //     (archivist-curated long-term memory). Each is gated on its own
    //     flag and capped at `USER_FILE_MAX_CHARS` (~1000 tokens) so a
    //     growing on-disk file can't push the system prompt out of the
    //     cache-friendly prefix range.
    //
    //     KV-cache contract: once these files land in a session's
    //     rendered prompt the bytes are frozen for the remainder of that
    //     session. Do not re-read them mid-turn — a byte change breaks
    //     the backend's automatic prefix cache. Mid-session writes to
    //     either file are intentionally only visible on the NEXT session.
    if options.include_profile {
        inject_workspace_file_capped(&mut out, workspace_dir, "PROFILE.md", USER_FILE_MAX_CHARS);
    }
    if options.include_memory_md {
        inject_workspace_file_capped(&mut out, workspace_dir, "MEMORY.md", USER_FILE_MAX_CHARS);
    }

    // 2. Filtered tool catalogue. Indices are taken in ascending order
    //    from `allowed_indices`, which itself preserves `parent_tools`
    //    order, so the rendering is deterministic. We use `.get(i)`
    //    defensively even though the current caller (subagent_runner)
    //    only produces in-range indices — a future caller that derives
    //    indices from a different source must not be able to panic this
    //    renderer with a stale index.
    //
    //    Rendering uses the caller-specified `tool_call_format` so
    //    sub-agents and the main dispatcher stay in lockstep.
    // Tool catalogue rendering is dispatcher-format-aware:
    //
    // - **Native**: The provider receives full tool schemas through
    //   the request body's `tools` field (via `filtered_specs` in the
    //   sub-agent runner) and emits structured `tool_calls`. Listing
    //   the same tools again as prose in the system prompt is pure
    //   duplication — for a skills_agent spawn with 62 dynamic gmail
    //   tools, that duplication added ~54k tokens and blew past the
    //   model's context window. We skip the prose `## Tools` section
    //   entirely in this mode.
    //
    // - **PFormat / Json**: Both are prompt-driven formats — the
    //   model discovers tools by reading the prose `## Tools` section
    //   and emits text-wrapped tool calls (`<tool_call>name[a|b]</tool_call>`
    //   for PFormat, `<tool_call>{"name":...}</tool_call>` for Json).
    //   Neither uses the native `tools` request field, so we MUST
    //   list each tool in prose — including dynamically-registered
    //   `extra_tools` — or the model has no way to know they exist.
    if !matches!(tool_call_format, ToolCallFormat::Native) {
        out.push_str("## Tools\n\n");
        let render_one = |out: &mut String, tool: &dyn Tool| match tool_call_format {
            ToolCallFormat::PFormat => {
                let sig = render_pformat_signature_for_box_tool(tool);
                let _ = writeln!(
                    out,
                    "- **{}**: {}\n  Call as: `{}`",
                    tool.name(),
                    tool.description(),
                    sig
                );
            }
            ToolCallFormat::Json => {
                let _ = writeln!(
                    out,
                    "- **{}**: {}\n  Parameters: `{}`",
                    tool.name(),
                    tool.description(),
                    tool.parameters_schema()
                );
            }
            ToolCallFormat::Native => {
                // Unreachable — outer guard skips Native entirely.
            }
        };
        for &i in allowed_indices {
            let Some(tool) = parent_tools.get(i) else {
                tracing::warn!(
                    index = i,
                    tool_count = parent_tools.len(),
                    "[context::prompt] dropping out-of-range tool index in subagent render"
                );
                continue;
            };
            render_one(&mut out, tool.as_ref());
        }
        for tool in extra_tools {
            render_one(&mut out, tool.as_ref());
        }
    }

    // 3. Sub-agent calling-convention preamble — format-aware.
    //    Sub-agents need the same call format the main dispatcher expects
    //    so their output parses correctly.
    out.push('\n');
    match tool_call_format {
        ToolCallFormat::PFormat => {
            out.push_str(
                "## Tool Use Protocol\n\n\
                 Tool calls use **P-Format**: compact, positional, pipe-delimited syntax \
                 wrapped in `<tool_call>` tags.\n\n\
                 ```\n<tool_call>\ntool_name[arg1|arg2]\n</tool_call>\n```\n\n\
                 Arguments are positional — match the order shown in each tool's `Call as:` \
                 signature above (alphabetical by parameter name). \
                 Escape `|` as `\\|`, `]` as `\\]` inside values. \
                 You may emit multiple `<tool_call>` blocks per response.\n\n\
                 Use the provided tools to accomplish the task. Reply with a concise, dense \
                 final answer when you have one — the parent agent will weave it back into the \
                 user-visible response.\n\n",
            );
        }
        ToolCallFormat::Json => {
            out.push_str(
                "## Tool Use Protocol\n\n\
                 To use a tool, wrap a JSON object in `<tool_call></tool_call>` tags:\n\n\
                 ```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n\
                 You may emit multiple `<tool_call>` blocks in a single response.\n\n\
                 Use the provided tools to accomplish the task. Reply with a concise, dense \
                 final answer when you have one — the parent agent will weave it back into the \
                 user-visible response.\n\n",
            );
        }
        ToolCallFormat::Native => {
            out.push_str(
                "Use the provided tools via the model's native tool-calling output. \
                 Reply with a concise, dense final answer when you have one — the parent \
                 agent will weave it back into the user-visible response.\n\n",
            );
        }
    }

    // 3b. Optional safety preamble. Definitions that do work with real
    //     side-effects (code_executor, tool_maker, skills_agent) set
    //     `omit_safety_preamble = false` so the narrow renderer used to
    //     silently drop that instruction — we now honour the flag.
    //     Byte-identical to `SafetySection::build`.
    if options.include_safety_preamble {
        out.push_str(
            "## Safety\n\n- Do not exfiltrate private data.\n- Do not run destructive commands without asking.\n- Do not bypass oversight or approval mechanisms.\n- Prefer `trash` over `rm`.\n- When in doubt, ask before acting externally.\n\n",
        );
    }

    // 3c/3d. `## Available Skills` and `## Connected Integrations`
    //        are no longer emitted here. Each agent that needs them
    //        renders its own block in its `prompt.rs` (skills_agent
    //        owns the executor voice, orchestrator/welcome own the
    //        delegator voice). Legacy Inline/File-sourced TOML agents
    //        that still route through this helper simply don't get
    //        either block — which matches the fact that none of them
    //        currently opt in.

    // 4. Workspace so the model knows where it is. Intentionally stable:
    //    no datetime, no hostname, no pid — see the KV-cache note above.
    let _ = writeln!(
        out,
        "## Workspace\n\nWorking directory: `{}`\n",
        workspace_dir.display()
    );

    // 6. Runtime banner — model name only. Stable for the lifetime of
    //    this sub-agent's definition.
    let _ = writeln!(out, "## Runtime\n\nModel: {model_name}");

    out
}

/// Ensure the workspace file is up-to-date with the compiled-in default.
///
/// On first install the file doesn't exist → write it. On subsequent runs
/// we store a hash of the compiled-in content in a sidecar file
/// (`.{filename}.builtin-hash`). If the hash changes (code was updated),
/// the disk file is overwritten so prompt improvements ship automatically.
/// User edits between code releases are preserved — we only overwrite when
/// the built-in default itself changes.
fn sync_workspace_file(workspace_dir: &Path, filename: &str) {
    let default_content = default_workspace_file_content(filename);
    if default_content.is_empty() {
        return;
    }

    let path = workspace_dir.join(filename);
    let hash_path = workspace_dir.join(format!(".{filename}.builtin-hash"));

    // Compute a simple hash of the current compiled-in content.
    let current_hash = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        default_content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    };

    // Read the last-written hash (if any).
    let stored_hash = std::fs::read_to_string(&hash_path).unwrap_or_default();

    if stored_hash.trim() == current_hash && path.exists() {
        // Built-in hasn't changed and file exists — nothing to do.
        return;
    }

    // Either first install, or the compiled-in default changed → write it.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, default_content) {
        log::warn!("[agent:prompt] failed to write workspace file {filename}: {e}");
        return;
    }
    let _ = std::fs::write(&hash_path, &current_hash);
    log::info!("[agent:prompt] updated workspace file {filename} (builtin content changed)");
}

/// Inject `filename` from `workspace_dir` into `prompt`, truncated to
/// [`BOOTSTRAP_MAX_CHARS`]. Thin wrapper around
/// [`inject_workspace_file_capped`] for bootstrap-class files
/// (`SOUL.md`, `IDENTITY.md`, `HEARTBEAT.md`).
fn inject_workspace_file(prompt: &mut String, workspace_dir: &Path, filename: &str) {
    inject_workspace_file_capped(prompt, workspace_dir, filename, BOOTSTRAP_MAX_CHARS);
}

/// Inject `filename` into `prompt` with an explicit character budget.
///
/// Used directly by callers that want a tighter cap than
/// [`BOOTSTRAP_MAX_CHARS`] — notably `PROFILE.md` and `MEMORY.md` which
/// are user-specific, potentially growing, and do not warrant a full
/// 20K-char budget (see [`USER_FILE_MAX_CHARS`]).
///
/// Missing / empty files are silently skipped so callers can inject
/// optional files unconditionally without emitting a noisy placeholder.
///
/// **KV-cache contract:** the output is a pure function of `filename`,
/// file bytes at call time, and `max_chars`. Callers must invoke this
/// once per session — re-reading mid-session breaks the inference
/// backend's automatic prefix cache. See the byte-stability note on
/// [`render_subagent_system_prompt`].
fn inject_workspace_file_capped(
    prompt: &mut String,
    workspace_dir: &Path,
    filename: &str,
    max_chars: usize,
) {
    let path = workspace_dir.join(filename);

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.chars().count() > max_chars {
                trimmed
                    .char_indices()
                    .nth(max_chars)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            prompt.push_str(truncated);
            if truncated.len() < trimmed.len() {
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
                );
            } else {
                prompt.push_str("\n\n");
            }
        }
        Err(_) => {
            // Keep prompt focused: missing optional identity/bootstrap files should not
            // add noisy placeholders that dilute tool-calling instructions.
        }
    }
}

fn default_workspace_file_content(filename: &str) -> &'static str {
    // The bundled identity files live at `src/openhuman/agent/prompts/`
    // (owned by the `agent/` tree because they describe agent identity).
    // This module is under `src/openhuman/context/`, so the relative path
    // walks up one level and back into `agent/prompts/`.
    match filename {
        "SOUL.md" => include_str!("SOUL.md"),
        "IDENTITY.md" => include_str!("IDENTITY.md"),
        "HEARTBEAT.md" => {
            "# Periodic Tasks\n\n# Add tasks below (one per line, starting with `- `)\n"
        }
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::LazyLock;

    static NO_FILTER: LazyLock<HashSet<String>> = LazyLock::new(HashSet::new);

    struct TestTool;

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            "test_tool"
        }

        fn description(&self) -> &str {
            "tool desc"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
            Ok(crate::openhuman::tools::ToolResult::success("ok"))
        }
    }

    #[test]
    fn prompt_builder_assembles_sections() {
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let prompt_tools = PromptTool::from_tools(&tools);
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "instr",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };
        let rendered = SystemPromptBuilder::with_defaults().build(&ctx).unwrap();
        assert!(rendered.contains("## Tools"));
        assert!(rendered.contains("test_tool"));
        assert!(rendered.contains("instr"));
    }

    #[test]
    fn identity_section_creates_missing_workspace_files() {
        let workspace =
            std::env::temp_dir().join(format!("openhuman_prompt_create_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![];
        let prompt_tools = PromptTool::from_tools(&tools);
        let ctx = PromptContext {
            workspace_dir: &workspace,
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };

        let section = IdentitySection;
        let _ = section.build(&ctx).unwrap();

        for file in ["SOUL.md", "IDENTITY.md", "HEARTBEAT.md"] {
            assert!(
                workspace.join(file).exists(),
                "expected workspace file to be created: {file}"
            );
        }
        let soul = std::fs::read_to_string(workspace.join("SOUL.md")).unwrap();
        assert!(
            soul.starts_with("# OpenHuman"),
            "SOUL.md should be seeded from src/openhuman/agent/prompts/SOUL.md"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn datetime_section_includes_timestamp_and_timezone() {
        let tools: Vec<Box<dyn Tool>> = vec![];
        let prompt_tools = PromptTool::from_tools(&tools);
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "instr",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };

        let rendered = DateTimeSection.build(&ctx).unwrap();
        assert!(rendered.starts_with("## Current Date & Time\n\n"));

        let payload = rendered.trim_start_matches("## Current Date & Time\n\n");
        assert!(payload.chars().any(|c| c.is_ascii_digit()));
        assert!(payload.contains(" ("));
        assert!(payload.ends_with(')'));
    }

    #[test]
    fn tools_section_pformat_renders_signature_not_schema() {
        // ToolsSection must render `name[arg1|arg2]` signatures when
        // `tool_call_format = PFormat`, NOT the verbose JSON schema —
        // that's where most of the prompt token saving comes from.
        struct ParamTool;
        #[async_trait]
        impl Tool for ParamTool {
            fn name(&self) -> &str {
                "make_tea"
            }
            fn description(&self) -> &str {
                "brew a cup of tea"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string" },
                        "sugar": { "type": "boolean" }
                    }
                })
            }
            async fn execute(
                &self,
                _args: serde_json::Value,
            ) -> anyhow::Result<crate::openhuman::tools::ToolResult> {
                Ok(crate::openhuman::tools::ToolResult::success("ok"))
            }
        }

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(ParamTool)];
        let prompt_tools = PromptTool::from_tools(&tools);
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };

        let rendered = ToolsSection.build(&ctx).unwrap();
        // Alphabetical: kind, sugar.
        assert!(
            rendered.contains("Call as: `make_tea[kind|sugar]`"),
            "expected p-format signature in tools section, got:\n{rendered}"
        );
        // Should NOT contain the raw JSON schema dump.
        assert!(
            !rendered.contains("\"properties\""),
            "tools section should drop the raw JSON schema in p-format mode, got:\n{rendered}"
        );
    }

    #[test]
    fn tools_section_json_renders_full_schema() {
        // The legacy `Json` mode must keep emitting full schemas so
        // existing prompts that depend on the verbose form are not
        // silently changed.
        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let prompt_tools = PromptTool::from_tools(&tools);
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::Json,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };

        let rendered = ToolsSection.build(&ctx).unwrap();
        assert!(
            rendered.contains("Parameters:"),
            "JSON mode should still print Parameters lines, got:\n{rendered}"
        );
        assert!(
            rendered.contains("\"type\""),
            "JSON mode should print the schema body, got:\n{rendered}"
        );
    }

    #[test]
    fn user_memory_section_renders_namespaces_with_headings() {
        let learned = LearnedContextData {
            tree_root_summaries: vec![
                ("user".into(), "Steven prefers terse Rust answers.".into()),
                (
                    "conversations".into(),
                    "Recent thread: prompt rework.".into(),
                ),
            ],
            ..Default::default()
        };
        let prompt_tools: Vec<PromptTool<'_>> = Vec::new();
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned,
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };
        let rendered = UserMemorySection.build(&ctx).unwrap();
        assert!(rendered.starts_with("## User Memory\n\n"));
        assert!(rendered.contains("### user\n\nSteven prefers terse Rust answers."));
        assert!(rendered.contains("### conversations\n\nRecent thread: prompt rework."));
    }

    #[test]
    fn user_memory_section_returns_empty_when_no_summaries() {
        // Empty learned context → section returns empty string and is
        // skipped by the prompt builder, so the cache boundary stays
        // exactly where it was for workspaces with no tree summaries.
        let learned = LearnedContextData::default();
        let prompt_tools: Vec<PromptTool<'_>> = Vec::new();
        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned,
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };
        let rendered = UserMemorySection.build(&ctx).unwrap();
        assert!(rendered.is_empty());
    }

    #[test]
    fn render_subagent_system_prompt_renders_workspace_tail() {
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_subagent_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are a focused sub-agent.",
            SubagentRenderOptions::narrow(),
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(rendered.contains("## Workspace"));
        assert!(rendered.contains("## Runtime"));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn subagent_render_options_invert_definition_flags() {
        // (omit_identity, omit_safety_preamble, omit_skills_catalog,
        //  omit_profile, omit_memory_md)
        let options = SubagentRenderOptions::from_definition_flags(true, false, true, false, false);
        assert!(!options.include_identity);
        assert!(options.include_safety_preamble);
        assert!(!options.include_skills_catalog);
        assert!(options.include_profile);
        assert!(options.include_memory_md);
        let narrow = SubagentRenderOptions::narrow();
        let default = SubagentRenderOptions::default();
        assert_eq!(narrow.include_identity, default.include_identity);
        assert_eq!(
            narrow.include_safety_preamble,
            default.include_safety_preamble
        );
        assert_eq!(
            narrow.include_skills_catalog,
            default.include_skills_catalog
        );
        assert_eq!(narrow.include_profile, default.include_profile);
        assert_eq!(narrow.include_memory_md, default.include_memory_md);
        // Narrow default = every flag off, including both user files.
        assert!(!narrow.include_profile);
        assert!(!narrow.include_memory_md);
    }

    #[test]
    fn render_subagent_system_prompt_honors_identity_safety_and_skills_flags() {
        let workspace =
            std::env::temp_dir().join(format!("openhuman_prompt_opts_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("SOUL.md"), "# Soul\nContext").unwrap();
        std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nContext").unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt_with_format(
            &workspace,
            "reasoning-v1",
            &[0],
            &tools,
            &[],
            "You are a specialist.",
            SubagentRenderOptions {
                include_identity: true,
                include_safety_preamble: true,
                include_skills_catalog: true,
                include_profile: false,
                include_memory_md: false,
            },
            ToolCallFormat::Json,
            &[],
        );

        assert!(rendered.contains("## Project Context"));
        assert!(rendered.contains("### SOUL.md"));
        assert!(rendered.contains("## Safety"));
        assert!(rendered.contains("## Available Skills"));
        // Json is a prompt-driven format (the model wraps JSON tool
        // calls in `<tool_call>` tags); it does NOT use the provider's
        // native function-calling channel. So the prose `## Tools`
        // section MUST still be rendered for Json, with each tool's
        // parameter schema inline so the model knows what to emit.
        // Only `ToolCallFormat::Native` gets the section omitted (see
        // the `native` branch below and the `!matches!(…, Native)`
        // guard in the renderer).
        assert!(rendered.contains("## Tools"));
        assert!(rendered.contains("Parameters:"));
        assert!(rendered.contains("\"type\""));

        let native = render_subagent_system_prompt_with_format(
            &workspace,
            "reasoning-v1",
            &[0],
            &tools,
            &[],
            "You are a specialist.",
            SubagentRenderOptions::narrow(),
            ToolCallFormat::Native,
            &[],
        );
        assert!(native.contains("native tool-calling output"));
        assert!(!native.contains("## Safety"));
        // Native is the only format where the prose `## Tools` section
        // is intentionally omitted — schemas travel through the
        // provider's `tools` field instead. Regression guard against
        // the ~54k-token schema duplication from the #447 PR.
        assert!(!native.contains("\n## Tools\n"));
        assert!(!native.contains("Parameters:"));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_system_prompt_injects_profile_md_even_when_identity_omitted() {
        // Regression: the welcome agent sets `omit_identity = true` to
        // drop the SOUL/IDENTITY preamble (it has its own voice) but it
        // still needs PROFILE.md to personalise the greeting. PROFILE.md
        // is gated on its own `include_profile` flag so the welcome path
        // can opt in without pulling SOUL/IDENTITY back in.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_profile_nosoul_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("SOUL.md"), "# Soul\nShould be hidden").unwrap();
        std::fs::write(
            workspace.join("IDENTITY.md"),
            "# Identity\nShould be hidden",
        )
        .unwrap();
        std::fs::write(
            workspace.join("PROFILE.md"),
            "# User Profile\nName: Jane Doe\nRole: Data scientist",
        )
        .unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are the welcome agent.",
            SubagentRenderOptions {
                include_identity: false,
                include_safety_preamble: false,
                include_skills_catalog: false,
                include_profile: true,
                include_memory_md: false,
            },
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            rendered.contains("### PROFILE.md"),
            "PROFILE.md header must appear when include_profile=true, got:\n{rendered}"
        );
        assert!(
            rendered.contains("Jane Doe"),
            "PROFILE.md body must be injected when include_profile=true, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("## Project Context"),
            "identity preamble must still be suppressed when include_identity=false"
        );
        assert!(
            !rendered.contains("### SOUL.md") && !rendered.contains("### IDENTITY.md"),
            "SOUL/IDENTITY must still be suppressed when include_identity=false"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_system_prompt_skips_profile_md_when_include_profile_false() {
        // Mirror of the opt-in regression above: narrow specialists
        // (planner, code_executor, critic, …) set `omit_profile = true`
        // and must NOT see PROFILE.md even when the file is on disk —
        // otherwise every sub-agent pays the token cost of onboarding
        // enrichment output that is irrelevant to their task.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_profile_opt_out_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("PROFILE.md"),
            "# User Profile\nName: Jane Doe\nRole: Data scientist",
        )
        .unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are a narrow specialist.",
            SubagentRenderOptions::narrow(), // include_profile defaults to false
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            !rendered.contains("### PROFILE.md"),
            "PROFILE.md must NOT appear when include_profile=false, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("Jane Doe"),
            "PROFILE.md body must NOT be leaked when include_profile=false"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_system_prompt_injects_profile_md_when_identity_included() {
        // When identity is on, PROFILE.md must still be injected alongside
        // SOUL/IDENTITY — the split must not regress the non-welcome path.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_profile_with_identity_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("SOUL.md"), "# Soul\nctx").unwrap();
        std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nctx").unwrap();
        std::fs::write(workspace.join("PROFILE.md"), "# User Profile\nhello").unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are a specialist.",
            SubagentRenderOptions {
                include_identity: true,
                include_safety_preamble: false,
                include_skills_catalog: false,
                include_profile: true,
                include_memory_md: false,
            },
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(rendered.contains("## Project Context"));
        assert!(rendered.contains("### SOUL.md"));
        assert!(rendered.contains("### IDENTITY.md"));
        assert!(rendered.contains("### PROFILE.md"));
        assert!(rendered.contains("hello"));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_system_prompt_silently_skips_missing_profile_md() {
        // Pre-onboarding workspaces have no PROFILE.md. The renderer must
        // not emit a noisy "[File not found: PROFILE.md]" placeholder or
        // an orphan "### PROFILE.md" header — the subagent prompt stays
        // focused on tools.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_profile_missing_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are the welcome agent.",
            SubagentRenderOptions::narrow(),
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            !rendered.contains("### PROFILE.md"),
            "empty/missing PROFILE.md should not emit a header, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("[File not found: PROFILE.md]"),
            "missing PROFILE.md should be silent, not a noisy placeholder"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn welcome_agent_definition_flags_still_load_profile_md() {
        // End-to-end-ish check against the real welcome agent flags: the
        // agent.toml sets omit_identity=true/omit_skills_catalog=true/
        // omit_safety_preamble=true/omit_profile=false. Mirror that exact
        // combo and verify PROFILE.md still lands in the rendered prompt.
        // If someone flips `omit_profile` back to its default (true), this
        // test breaks.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_welcome_flags_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("PROFILE.md"),
            "# User Profile\nTimezone: PST\nRole: Crypto trader",
        )
        .unwrap();

        // Match `src/openhuman/agent/agents/welcome/agent.toml` exactly.
        let options = SubagentRenderOptions::from_definition_flags(
            true,  // omit_identity
            true,  // omit_safety_preamble
            true,  // omit_skills_catalog
            false, // omit_profile   — welcome opts IN to PROFILE.md
            false, // omit_memory_md — welcome opts IN to MEMORY.md too
        );

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "# Welcome Agent\n\nYou are the welcome agent.",
            options,
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            rendered.contains("### PROFILE.md"),
            "welcome agent (omit_profile=false) must load PROFILE.md, got:\n{rendered}"
        );
        assert!(
            rendered.contains("Crypto trader"),
            "PROFILE.md body must reach the welcome agent prompt"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn narrow_subagent_definition_flags_skip_profile_md() {
        // Inverse of `welcome_agent_definition_flags_still_load_profile_md`:
        // a narrow specialist (e.g. `code_executor`, `critic`) leaves
        // `omit_profile` at its default `true`. PROFILE.md must NOT be
        // injected even when present on disk — the narrow runner is
        // task-focused and should not pay the token cost.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_narrow_flags_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("PROFILE.md"),
            "# User Profile\nTimezone: PST\nRole: Crypto trader",
        )
        .unwrap();

        // Mirrors e.g. `critic/agent.toml` — all omit_* default-true.
        let options = SubagentRenderOptions::from_definition_flags(true, true, true, true, true);

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are a narrow specialist.",
            options,
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            !rendered.contains("### PROFILE.md"),
            "narrow specialist (omit_profile=true) must NOT load PROFILE.md, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("Crypto trader"),
            "narrow specialist must not leak PROFILE.md body"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_system_prompt_injects_memory_md_when_enabled() {
        // Opt-in agents with `omit_memory_md = false` must see MEMORY.md
        // (archivist-curated long-term memory) in their rendered prompt.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_memory_on_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("MEMORY.md"),
            "# Long-term memory\nUser prefers terse Rust answers.",
        )
        .unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are the welcome agent.",
            SubagentRenderOptions {
                include_identity: false,
                include_safety_preamble: false,
                include_skills_catalog: false,
                include_profile: false,
                include_memory_md: true,
            },
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            rendered.contains("### MEMORY.md"),
            "MEMORY.md header must appear when include_memory_md=true, got:\n{rendered}"
        );
        assert!(
            rendered.contains("terse Rust answers"),
            "MEMORY.md body must be injected when include_memory_md=true"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn render_subagent_system_prompt_skips_memory_md_when_disabled() {
        // Narrow specialists with `omit_memory_md = true` (the default)
        // must NOT see MEMORY.md even when it exists on disk.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_memory_off_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("MEMORY.md"),
            "# Long-term memory\nUser prefers terse Rust answers.",
        )
        .unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are a narrow specialist.",
            SubagentRenderOptions::narrow(),
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(
            !rendered.contains("### MEMORY.md"),
            "MEMORY.md must NOT appear when include_memory_md=false, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("terse Rust answers"),
            "MEMORY.md body must not leak when include_memory_md=false"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn profile_md_and_memory_md_are_capped_at_user_file_max_chars() {
        // Both PROFILE.md and MEMORY.md are user-specific files that can
        // grow over time. Injection caps them at USER_FILE_MAX_CHARS
        // (~1000 tokens each) so the system prompt footprint stays
        // bounded. Test both files at once to pin the shared budget.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_user_cap_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let big = "x".repeat(USER_FILE_MAX_CHARS + 500);
        std::fs::write(workspace.join("PROFILE.md"), &big).unwrap();
        std::fs::write(workspace.join("MEMORY.md"), &big).unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are the orchestrator.",
            SubagentRenderOptions {
                include_identity: false,
                include_safety_preamble: false,
                include_skills_catalog: false,
                include_profile: true,
                include_memory_md: true,
            },
            ToolCallFormat::PFormat,
            &[],
        );

        assert!(rendered.contains("### PROFILE.md"));
        assert!(rendered.contains("### MEMORY.md"));
        // Each file gets its own truncation marker mentioning the cap.
        let marker = format!("[... truncated at {USER_FILE_MAX_CHARS} chars");
        assert_eq!(
            rendered.matches(marker.as_str()).count(),
            2,
            "both PROFILE.md and MEMORY.md must emit the truncation marker at \
             USER_FILE_MAX_CHARS — found:\n{rendered}"
        );
        // Sanity-check the cap is genuinely tighter than the bootstrap cap.
        assert!(USER_FILE_MAX_CHARS < BOOTSTRAP_MAX_CHARS);

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn rendered_subagent_system_prompt_is_byte_stable_across_repeat_calls() {
        // KV-cache contract: two spawns of the same sub-agent definition
        // against the same workspace must produce byte-identical system
        // prompts. If PROFILE.md or MEMORY.md are re-read with a
        // different-typed truncation path, or if either cap drifts, the
        // bytes differ and the backend's automatic prefix cache busts.
        // This test pins the invariant end-to-end.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_byte_stable_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("PROFILE.md"), "# User Profile\nJane Doe").unwrap();
        std::fs::write(workspace.join("MEMORY.md"), "# Memory\nRecent: shipped v1").unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let opts = SubagentRenderOptions {
            include_identity: false,
            include_safety_preamble: false,
            include_skills_catalog: false,
            include_profile: true,
            include_memory_md: true,
        };

        let first = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are the orchestrator.",
            opts,
            ToolCallFormat::PFormat,
            &[],
        );
        let second = render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            &[],
            "You are the orchestrator.",
            opts,
            ToolCallFormat::PFormat,
            &[],
        );

        assert_eq!(
            first, second,
            "repeat spawns must produce byte-identical prompts"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn for_subagent_builder_injects_user_files_even_when_identity_omitted() {
        // Regression pin for the review finding: the runtime Tauri chat
        // path spins welcome/trigger_* via `Agent::from_config_for_agent`
        // → `SystemPromptBuilder::for_subagent(body, omit_identity=true, …)`,
        // which deliberately drops `IdentitySection`. Before
        // `UserFilesSection` existed, our PROFILE/MEMORY injection lived
        // inside `IdentitySection::build` and got dropped along with it,
        // so the first Tauri turn never saw the user's onboarding output
        // even though the subagent_runner path and the debug dumper did.
        //
        // This test exercises the exact builder call-site the runtime
        // uses for welcome (`omit_identity = true`, both user-file flags
        // opted in via PromptContext) and pins that the rendered prompt
        // contains both files.
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_for_subagent_user_files_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("PROFILE.md"),
            "# User Profile\nJane Doe — crypto trader in PST.",
        )
        .unwrap();
        std::fs::write(
            workspace.join("MEMORY.md"),
            "# Long-term memory\nShipped v1 last sprint; prefers terse Rust.",
        )
        .unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![];
        let prompt_tools = PromptTool::from_tools(&tools);
        let ctx = PromptContext {
            workspace_dir: &workspace,
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: true,
            include_memory_md: true,
        };

        // Mirror the welcome agent runtime path:
        // `SystemPromptBuilder::for_subagent(body, omit_identity=true, …)`.
        let builder = SystemPromptBuilder::for_subagent(
            "You are the welcome agent.".into(),
            true, // omit_identity  — drops SOUL/IDENTITY preamble
            true, // omit_safety_preamble
            true, // omit_skills_catalog
        );
        let rendered = builder.build(&ctx).unwrap();

        assert!(
            !rendered.contains("## Project Context"),
            "identity preamble must still be suppressed when omit_identity=true"
        );
        assert!(
            rendered.contains("### PROFILE.md") && rendered.contains("Jane Doe"),
            "welcome runtime path must inject PROFILE.md despite omit_identity=true, got:\n{rendered}"
        );
        assert!(
            rendered.contains("### MEMORY.md") && rendered.contains("terse Rust"),
            "welcome runtime path must inject MEMORY.md despite omit_identity=true, got:\n{rendered}"
        );

        // Mirror the narrow-specialist runtime path (code_executor,
        // critic, …): both flags off → user files must stay out.
        let ctx_narrow = PromptContext {
            workspace_dir: &workspace,
            model_name: "test-model",
            agent_id: "",
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };
        let narrow = builder.build(&ctx_narrow).unwrap();
        assert!(
            !narrow.contains("### PROFILE.md") && !narrow.contains("### MEMORY.md"),
            "narrow specialist runtime path must NOT leak user files, got:\n{narrow}"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn sync_workspace_file_updates_hash_and_inject_workspace_file_truncates() {
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_workspace_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();

        sync_workspace_file(&workspace, "SOUL.md");
        let hash_path = workspace.join(".SOUL.md.builtin-hash");
        assert!(workspace.join("SOUL.md").exists());
        assert!(hash_path.exists());
        let original_hash = std::fs::read_to_string(&hash_path).unwrap();

        std::fs::write(workspace.join("SOUL.md"), "user override").unwrap();
        sync_workspace_file(&workspace, "SOUL.md");
        assert_eq!(std::fs::read_to_string(&hash_path).unwrap(), original_hash);
        assert_eq!(
            std::fs::read_to_string(workspace.join("SOUL.md")).unwrap(),
            "user override"
        );

        std::fs::write(
            workspace.join("BIG.md"),
            "x".repeat(BOOTSTRAP_MAX_CHARS + 50),
        )
        .unwrap();
        let mut prompt = String::new();
        inject_workspace_file(&mut prompt, &workspace, "BIG.md");
        assert!(prompt.contains("### BIG.md"));
        assert!(prompt.contains("[... truncated at"));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn prompt_tool_constructors_and_user_memory_skip_empty_bodies() {
        let plain = PromptTool::new("shell", "run commands");
        assert_eq!(plain.name, "shell");
        assert!(plain.parameters_schema.is_none());

        let with_schema =
            PromptTool::with_schema("http_request", "fetch data", "{\"type\":\"object\"}".into());
        assert_eq!(
            with_schema.parameters_schema.as_deref(),
            Some("{\"type\":\"object\"}")
        );

        let ctx = PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "model",
            agent_id: "",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData {
                tree_root_summaries: vec![
                    ("user".into(), "kept".into()),
                    ("empty".into(), "   ".into()),
                ],
                ..Default::default()
            },
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        };
        let rendered = UserMemorySection.build(&ctx).unwrap();
        assert!(rendered.contains("### user"));
        assert!(!rendered.contains("### empty"));
        assert_eq!(default_workspace_file_content("missing"), "");
    }
}
