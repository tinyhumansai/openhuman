pub mod types;
pub use types::*;
mod connected_identities;
pub use connected_identities::render_connected_identities;

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
        // them (integrations_agent for the skill-executor voice,
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

    /// Build from a [`PromptSource::Dynamic`] function pointer.
    ///
    /// The function is called every time [`Self::build`] runs, with the
    /// live [`PromptContext`] the call-site supplies — so late-arriving
    /// state like `connected_integrations` (fetched asynchronously at
    /// the start of a session) reaches the dynamic renderer instead of
    /// being frozen into an empty slice at builder-construction time.
    ///
    /// KV-cache contract: callers must only invoke `build_system_prompt`
    /// once per session (after `fetch_connected_integrations`). The
    /// rendered bytes are then frozen for the rest of the session the
    /// same way `from_final_body` freezes them — the difference is just
    /// *when* the freeze happens.
    pub fn from_dynamic(
        builder: crate::openhuman::agent::harness::definition::PromptBuilder,
    ) -> Self {
        Self {
            sections: vec![Box::new(DynamicPromptSection::new(builder))],
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

/// Section that defers to a [`crate::openhuman::agent::harness::definition::PromptBuilder`]
/// every time it renders, so dynamic prompts (orchestrator, welcome,
/// integrations_agent, …) get to see the live runtime
/// [`PromptContext`] — including `connected_integrations`, which are
/// fetched asynchronously after the builder itself has been
/// constructed.
pub struct DynamicPromptSection {
    builder: crate::openhuman::agent::harness::definition::PromptBuilder,
}

impl DynamicPromptSection {
    pub fn new(builder: crate::openhuman::agent::harness::definition::PromptBuilder) -> Self {
        Self { builder }
    }
}

impl PromptSection for DynamicPromptSection {
    fn name(&self) -> &str {
        "dynamic_prompt"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        (self.builder)(ctx)
    }
}

pub struct IdentitySection;
pub struct ToolsSection;
pub struct SafetySection;
// `SkillsSection` and `ConnectedIntegrationsSection` previously lived
// here and branched on `ctx.agent_id` to pick between the skill-
// executor and delegator voice. They've been removed — each agent's
// `prompt.rs` now renders its own block inline (integrations_agent owns the
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

            // One rendering shape for every dispatcher: a compact
            // P-Format signature (`name[a|b|c]`). The signature comes
            // straight from the parameter schema (alphabetical by
            // property name — see `pformat` module docs for why) so
            // model and parser agree on argument ordering. For
            // `Native` dispatchers the provider already has the full
            // JSON schema in the API request, so repeating it in the
            // prompt is pure token bloat; for `Json` / `PFormat` text
            // dispatchers the dispatcher's own `prompt_instructions`
            // block (appended below) carries whatever schema detail
            // the wire format needs.
            let signature = render_pformat_signature_for_prompt(tool);
            let _ = writeln!(
                out,
                "- **{}**: {}\n  Call as: `{}`",
                tool.name, tool.description, signature
            );
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
// gone — `## Available Skills` lives in `integrations_agent/prompt.rs`, and
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
        connected_identities_md: String::new(),
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
    _connected_integrations: &[ConnectedIntegration],
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
    //   duplication — for a integrations_agent spawn with 62 dynamic gmail
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
    //     side-effects (code_executor, tool_maker, integrations_agent) set
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
    //        renders its own block in its `prompt.rs` (integrations_agent
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
    let stored_hash = stored_hash.trim();

    if stored_hash == current_hash && path.exists() {
        // Built-in hasn't changed and file exists — nothing to do.
        return;
    }

    // Decide whether to overwrite the existing file. Two safe cases:
    //   1. File doesn't exist yet — first install, write the default.
    //   2. File exists AND its current hash matches the stored builtin
    //      hash — the user hasn't edited it since we last wrote it, so
    //      it's safe to ship the new default.
    // Otherwise the file has been hand-edited between releases; leave
    // the user's version in place and just update the stored hash so we
    // stop re-comparing against the old default on every boot.
    let file_exists = path.exists();
    let user_unmodified = if file_exists {
        match std::fs::read_to_string(&path) {
            Ok(disk) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                disk.hash(&mut hasher);
                let disk_hash = format!("{:016x}", hasher.finish());
                disk_hash == stored_hash
            }
            Err(_) => false,
        }
    } else {
        false
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if !file_exists || user_unmodified {
        if let Err(e) = std::fs::write(&path, default_content) {
            log::warn!("[agent:prompt] failed to write workspace file {filename}: {e}");
            return;
        }
        log::info!("[agent:prompt] updated workspace file {filename} (builtin content changed)");
    } else {
        log::info!(
            "[agent:prompt] keeping user-edited workspace file {filename} (builtin changed but disk contents diverge)"
        );
    }
    let _ = std::fs::write(&hash_path, &current_hash);
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
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => {
                // Keep prompt focused: missing optional identity/bootstrap files should not
                // add noisy placeholders that dilute tool-calling instructions.
            }
            _ => {
                log::debug!("[prompt] failed to read {}: {e}", path.display());
            }
        },
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
#[path = "mod_tests.rs"]
mod tests;
