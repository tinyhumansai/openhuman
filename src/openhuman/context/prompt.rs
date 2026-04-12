use crate::openhuman::skills::Skill;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use chrono::Local;
use std::fmt::Write;
use std::path::Path;

const BOOTSTRAP_MAX_CHARS: usize = 20_000;
const CACHE_BOUNDARY_MARKER: &str = "<!-- CACHE_BOUNDARY -->";

/// Per-namespace cap when injecting tree summarizer root summaries into
/// the prompt. ~8 000 chars ≈ 2 000 tokens — that's the floor the user
/// asked for ("at least 2000 tokens of user memory") for a single
/// namespace, and matches what the tree summarizer's `Day` level
/// already enforces upstream.
pub(crate) const USER_MEMORY_PER_NAMESPACE_MAX_CHARS: usize = 8_000;

/// Hard ceiling across all namespaces, so a workspace with 30 namespaces
/// doesn't burn the entire context window. ~32 000 chars ≈ 8 000 tokens.
pub(crate) const USER_MEMORY_TOTAL_MAX_CHARS: usize = 32_000;

/// Pre-fetched learned context data for prompt sections (avoids blocking the runtime).
#[derive(Debug, Clone, Default)]
pub struct LearnedContextData {
    /// Recent observations from the learning subsystem.
    pub observations: Vec<String>,
    /// Recognized patterns.
    pub patterns: Vec<String>,
    /// Learned user profile entries.
    pub user_profile: Vec<String>,
    /// Pre-fetched root-level summaries from the tree summarizer, one per
    /// namespace that has a root node on disk.
    ///
    /// Each entry is `(namespace, body)`. The body is the markdown body of
    /// `memory/namespaces/{ns}/tree/root.md` — already truncated to a
    /// per-namespace cap by the fetcher so the section can render without
    /// any further sizing logic.
    ///
    /// Empty when the tree summarizer has never run on this workspace
    /// (the section then renders nothing and is dropped from the prompt).
    pub tree_root_summaries: Vec<(String, String)>,
}

/// A lightweight tool descriptor for prompt rendering.
///
/// Shared shape so every call-site that builds a system prompt — main
/// agents (which own `Box<dyn Tool>`), sub-agents, and channel runtimes
/// (which only have `(name, description)` tuples from their tool
/// registries) — can feed the same [`ToolsSection`] implementation
/// instead of each writing its own. Callers adapt their own tool
/// representation into a `Vec<PromptTool<'_>>` at the PromptContext
/// construction site via a one-line `.iter().map(...).collect()` or via
/// [`PromptTool::from_tools`].
///
/// `parameters_schema` is optional because channel runtimes don't have
/// full JSON schemas at prompt-build time; the tools section renders
/// the schema line only when it's present.
#[derive(Debug, Clone)]
pub struct PromptTool<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub parameters_schema: Option<String>,
}

impl<'a> PromptTool<'a> {
    pub fn new(name: &'a str, description: &'a str) -> Self {
        Self {
            name,
            description,
            parameters_schema: None,
        }
    }

    pub fn with_schema(name: &'a str, description: &'a str, parameters_schema: String) -> Self {
        Self {
            name,
            description,
            parameters_schema: Some(parameters_schema),
        }
    }

    /// Adapt a `Box<dyn Tool>` slice into a `Vec<PromptTool<'_>>`. The
    /// returned vector borrows names and descriptions from the original
    /// tools, so it must not outlive them. Main-agent call-sites use
    /// this one-liner to build the slice passed into [`PromptContext::tools`].
    pub fn from_tools(tools: &'a [Box<dyn Tool>]) -> Vec<PromptTool<'a>> {
        tools
            .iter()
            .map(|t| PromptTool {
                name: t.name(),
                description: t.description(),
                parameters_schema: Some(t.parameters_schema().to_string()),
            })
            .collect()
    }
}

/// How the [`ToolsSection`] should render each tool entry. Driven by
/// the dispatcher choice on the agent — JSON-schema rendering is the
/// historic format; P-Format is the new default text protocol.
///
/// `Native` is for providers that ship structured tool calls directly,
/// in which case the catalogue body is informational only and the
/// renderer falls back to JSON-schema (it's the most descriptive form).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolCallFormat {
    /// `tool_name[arg1|arg2|...]` — compact, positional, ~80% fewer
    /// tokens than JSON. The default.
    #[default]
    PFormat,
    /// Legacy JSON-in-tag rendering. Each tool entry shows the full
    /// JSON schema. Kept for backwards compatibility with prompts
    /// tuned against the old format.
    Json,
    /// Provider supplies structured tool calls — the catalogue is
    /// informational. Renders in the same JSON-schema form as `Json`.
    Native,
}

pub struct PromptContext<'a> {
    pub workspace_dir: &'a Path,
    pub model_name: &'a str,
    pub tools: &'a [PromptTool<'a>],
    pub skills: &'a [Skill],
    pub dispatcher_instructions: &'a str,
    /// Pre-fetched learned context (empty when learning is disabled).
    pub learned: LearnedContextData,
    /// When non-empty, only tools in this set are rendered in the prompt.
    /// Skills section is omitted when a filter is active (the main agent
    /// delegates skill work to sub-agents).
    pub visible_tool_names: &'a std::collections::HashSet<String>,
    /// How [`ToolsSection`] should render each tool entry. Defaults to
    /// [`ToolCallFormat::PFormat`] when not set.
    pub tool_call_format: ToolCallFormat,
}

pub trait PromptSection: Send + Sync {
    fn name(&self) -> &str;
    fn build(&self, ctx: &PromptContext<'_>) -> Result<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPrompt {
    pub text: String,
    pub cache_boundary: Option<usize>,
}

#[derive(Default)]
pub struct SystemPromptBuilder {
    sections: Vec<Box<dyn PromptSection>>,
}

impl SystemPromptBuilder {
    pub fn with_defaults() -> Self {
        Self {
            sections: vec![
                Box::new(IdentitySection),
                // User memory sits right after the identity bootstrap so the
                // model has rich, persistent context about the user before it
                // sees the tool catalogue. Section is empty (and skipped) when
                // the tree summarizer has nothing on disk yet.
                Box::new(UserMemorySection),
                Box::new(ToolsSection),
                Box::new(SafetySection),
                Box::new(SkillsSection),
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
        omit_skills_catalog: bool,
    ) -> Self {
        let mut sections: Vec<Box<dyn PromptSection>> =
            vec![Box::new(ArchetypePromptSection::new(archetype_prompt_text))];

        if !omit_identity {
            sections.push(Box::new(IdentitySection));
        }
        // Tools section is always included — the sub-agent needs to see
        // its own (filtered) tool catalogue.
        sections.push(Box::new(ToolsSection));
        if !omit_safety_preamble {
            sections.push(Box::new(SafetySection));
        }
        if !omit_skills_catalog {
            sections.push(Box::new(SkillsSection));
        }
        sections.push(Box::new(WorkspaceSection));

        Self { sections }
    }

    pub fn add_section(mut self, section: Box<dyn PromptSection>) -> Self {
        self.sections.push(section);
        self
    }

    pub fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.build_with_cache_metadata(ctx)?.text)
    }

    pub fn build_with_cache_metadata(&self, ctx: &PromptContext<'_>) -> Result<RenderedPrompt> {
        let mut output = String::new();
        let mut cache_boundary_inserted = false;
        for section in &self.sections {
            let part = section.build(ctx)?;
            if part.trim().is_empty() {
                continue;
            }
            // Insert cache boundary marker before the first dynamic section.
            // Static sections (identity, tools, safety, skills) are cacheable;
            // dynamic sections (workspace, datetime, runtime) change per request.
            if !cache_boundary_inserted && is_dynamic_section(section.name()) {
                output.push_str(CACHE_BOUNDARY_MARKER);
                output.push_str("\n\n");
                cache_boundary_inserted = true;
            }
            output.push_str(part.trim_end());
            output.push_str("\n\n");
        }
        Ok(extract_cache_boundary(&output))
    }
}

pub fn extract_cache_boundary(rendered: &str) -> RenderedPrompt {
    if let Some(marker_idx) = rendered.find(CACHE_BOUNDARY_MARKER) {
        let mut text = rendered.to_string();
        let end = marker_idx + CACHE_BOUNDARY_MARKER.len();
        text.replace_range(marker_idx..end, "");
        if text[marker_idx..].starts_with("\n\n") {
            text.replace_range(marker_idx..marker_idx + 2, "");
        }
        return RenderedPrompt {
            text,
            cache_boundary: Some(marker_idx),
        };
    }

    RenderedPrompt {
        text: rendered.to_string(),
        cache_boundary: None,
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
pub struct SkillsSection;
pub struct WorkspaceSection;
pub struct RuntimeSection;
pub struct DateTimeSection;
pub struct UserMemorySection;

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
        let all_files: &[&str] = &["SOUL.md", "IDENTITY.md", "USER.md", "HEARTBEAT.md"];
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

        Ok(prompt)
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

impl PromptSection for SkillsSection {
    fn name(&self) -> &str {
        "skills"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        // When a visible-tool filter is active the main agent delegates
        // all skill work to sub-agents — skip the skills catalog.
        if ctx.skills.is_empty() || !ctx.visible_tool_names.is_empty() {
            return Ok(String::new());
        }

        let mut prompt = String::from("## Available Skills\n\n<available_skills>\n");
        for skill in ctx.skills {
            let location = skill.location.clone().unwrap_or_else(|| {
                ctx.workspace_dir
                    .join("skills")
                    .join(&skill.name)
                    .join("SKILL.md")
            });
            let _ = writeln!(
                prompt,
                "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n    <location>{}</location>\n  </skill>",
                skill.name,
                skill.description,
                location.display()
            );
        }
        prompt.push_str("</available_skills>");
        Ok(prompt)
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

/// Returns true for sections whose content changes between requests.
/// Static sections (identity, tools, safety, skills) are placed before
/// the cache boundary; dynamic sections (workspace, datetime, runtime) after.
fn is_dynamic_section(name: &str) -> bool {
    matches!(name, "workspace" | "datetime" | "runtime")
}

/// Per-definition rendering flags passed into
/// [`render_subagent_system_prompt`]. Mirrors the `omit_*` fields on
/// [`crate::openhuman::agent::harness::definition::AgentDefinition`] so
/// the runner can thread each definition's preferences through without
/// growing the function signature.
///
/// KV-cache-stable as long as the flags are read from a definition that
/// does not change mid-session.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubagentRenderOptions {
    /// When `false`, include the standard `## Safety` block. Mirrors
    /// `AgentDefinition::omit_safety_preamble`. Defaults to `false`
    /// (omit) because the narrow sub-agent renderer historically
    /// skipped this section entirely.
    pub include_safety_preamble: bool,
    /// When `false`, skip the identity/project-context dump. Mirrors
    /// `AgentDefinition::omit_identity`. Defaults to `false`; setting
    /// this to `true` is uncommon because sub-agents usually run
    /// narrow and the identity block pushes too many tokens.
    pub include_identity: bool,
    /// When `false`, skip the skills catalogue. Mirrors
    /// `AgentDefinition::omit_skills_catalog`. Defaults to `false`
    /// for the same reason as `include_identity`.
    pub include_skills_catalog: bool,
}

impl SubagentRenderOptions {
    /// Build the narrow default (every section off) — matches the
    /// historical behaviour of the purpose-built renderer before the
    /// flags were threaded through.
    pub fn narrow() -> Self {
        Self::default()
    }

    /// Construct from the per-definition flags, inverting them into the
    /// positive-sense `include_*` shape used by the renderer.
    pub fn from_definition_flags(
        omit_identity: bool,
        omit_safety_preamble: bool,
        omit_skills_catalog: bool,
    ) -> Self {
        Self {
            include_identity: !omit_identity,
            include_safety_preamble: !omit_safety_preamble,
            include_skills_catalog: !omit_skills_catalog,
        }
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
    archetype_body: &str,
    options: SubagentRenderOptions,
) -> String {
    render_subagent_system_prompt_with_format(
        workspace_dir,
        model_name,
        allowed_indices,
        parent_tools,
        archetype_body,
        options,
        ToolCallFormat::PFormat,
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
    archetype_body: &str,
    options: SubagentRenderOptions,
    tool_call_format: ToolCallFormat,
) -> String {
    let mut out = String::new();

    // 1. Archetype role prompt. Works for both `PromptSource::Inline`
    //    and `PromptSource::File` because the caller preloaded the
    //    body via `load_prompt_source`.
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
        for file in &["SOUL.md", "IDENTITY.md", "USER.md"] {
            inject_workspace_file(&mut out, workspace_dir, file);
        }
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
    out.push_str("## Tools\n\n");
    for &i in allowed_indices {
        let Some(tool) = parent_tools.get(i) else {
            tracing::warn!(
                index = i,
                tool_count = parent_tools.len(),
                "[context::prompt] dropping out-of-range tool index in subagent render"
            );
            continue;
        };
        match tool_call_format {
            ToolCallFormat::PFormat => {
                let sig = render_pformat_signature_for_box_tool(tool.as_ref());
                let _ = writeln!(
                    out,
                    "- **{}**: {}\n  Call as: `{}`",
                    tool.name(),
                    tool.description(),
                    sig
                );
            }
            ToolCallFormat::Json | ToolCallFormat::Native => {
                let _ = writeln!(
                    out,
                    "- **{}**: {}\n  Parameters: `{}`",
                    tool.name(),
                    tool.description(),
                    tool.parameters_schema()
                );
            }
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

    // 3c. Optional skills catalogue. Off by default because sub-agents
    //     usually skip skills entirely. Kept here so a custom
    //     definition can opt in without falling back to the general
    //     builder. The renderer intentionally takes no `skills` slice
    //     — the caller would have to extend this helper before
    //     enabling this flag for real, which keeps the common (narrow)
    //     path free of extra arguments.
    if options.include_skills_catalog {
        out.push_str("## Available Skills\n\n");
        out.push_str(
            "Skills are loaded on demand. Use `read` on the skill path to get full instructions.\n\n",
        );
    }

    // 4. Insert the cache boundary before the dynamic tail. Typed
    //    sub-agents keep the narrow/static instructions above this
    //    marker and thread the resulting byte offset through the
    //    provider request so repeat spawns can reuse prompt prefill.
    out.push_str(CACHE_BOUNDARY_MARKER);
    out.push_str("\n\n");

    // 5. Workspace so the model knows where it is. Intentionally stable:
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
        use std::hash::{Hash, Hasher};
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

fn inject_workspace_file(prompt: &mut String, workspace_dir: &Path, filename: &str) {
    let path = workspace_dir.join(filename);

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.chars().count() > BOOTSTRAP_MAX_CHARS {
                trimmed
                    .char_indices()
                    .nth(BOOTSTRAP_MAX_CHARS)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            prompt.push_str(truncated);
            if truncated.len() < trimmed.len() {
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {BOOTSTRAP_MAX_CHARS} chars — use `read` for full file]\n"
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
        "SOUL.md" => include_str!("../agent/prompts/SOUL.md"),
        "IDENTITY.md" => include_str!("../agent/prompts/IDENTITY.md"),
        "USER.md" => include_str!("../agent/prompts/USER.md"),
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "instr",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
        };
        let rendered = SystemPromptBuilder::with_defaults()
            .build_with_cache_metadata(&ctx)
            .unwrap();
        assert!(rendered.text.contains("## Tools"));
        assert!(rendered.text.contains("test_tool"));
        assert!(rendered.text.contains("instr"));
        assert!(!rendered.text.contains(CACHE_BOUNDARY_MARKER));
        assert!(rendered.cache_boundary.is_some());
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
        };

        let section = IdentitySection;
        let _ = section.build(&ctx).unwrap();

        for file in ["SOUL.md", "IDENTITY.md", "USER.md", "HEARTBEAT.md"] {
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "instr",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
        };

        let rendered = DateTimeSection.build(&ctx).unwrap();
        assert!(rendered.starts_with("## Current Date & Time\n\n"));

        let payload = rendered.trim_start_matches("## Current Date & Time\n\n");
        assert!(payload.chars().any(|c| c.is_ascii_digit()));
        assert!(payload.contains(" ("));
        assert!(payload.ends_with(')'));
    }

    #[test]
    fn extract_cache_boundary_removes_marker_and_returns_offset() {
        let rendered = extract_cache_boundary("static\n\n<!-- CACHE_BOUNDARY -->\n\ndynamic\n");
        assert_eq!(rendered.text, "static\n\ndynamic\n");
        assert_eq!(rendered.cache_boundary, Some("static\n\n".len()));
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::Json,
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned,
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
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
            tools: &prompt_tools,
            skills: &[],
            dispatcher_instructions: "",
            learned,
            visible_tool_names: &NO_FILTER,
            tool_call_format: ToolCallFormat::PFormat,
        };
        let rendered = UserMemorySection.build(&ctx).unwrap();
        assert!(rendered.is_empty());
    }

    #[test]
    fn render_subagent_system_prompt_includes_cache_boundary_before_workspace() {
        let workspace = std::env::temp_dir().join(format!(
            "openhuman_prompt_subagent_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = extract_cache_boundary(&render_subagent_system_prompt(
            &workspace,
            "test-model",
            &[0],
            &tools,
            "You are a focused sub-agent.",
            SubagentRenderOptions::narrow(),
        ));

        assert!(
            rendered.cache_boundary.is_some(),
            "typed sub-agent prompts should expose an explicit cache boundary"
        );
        assert!(rendered.text.contains("## Workspace"));
        assert!(!rendered.text.contains(CACHE_BOUNDARY_MARKER));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn extract_cache_boundary_without_marker_returns_original_text() {
        let rendered = extract_cache_boundary("hello");
        assert_eq!(rendered.text, "hello");
        assert_eq!(rendered.cache_boundary, None);
    }

    #[test]
    fn subagent_render_options_invert_definition_flags() {
        let options = SubagentRenderOptions::from_definition_flags(true, false, true);
        assert!(!options.include_identity);
        assert!(options.include_safety_preamble);
        assert!(!options.include_skills_catalog);
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
    }

    #[test]
    fn render_subagent_system_prompt_honors_identity_safety_and_skills_flags() {
        let workspace =
            std::env::temp_dir().join(format!("openhuman_prompt_opts_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("SOUL.md"), "# Soul\nContext").unwrap();
        std::fs::write(workspace.join("IDENTITY.md"), "# Identity\nContext").unwrap();
        std::fs::write(workspace.join("USER.md"), "# User\nContext").unwrap();

        let tools: Vec<Box<dyn Tool>> = vec![Box::new(TestTool)];
        let rendered = render_subagent_system_prompt_with_format(
            &workspace,
            "reasoning-v1",
            &[0],
            &tools,
            "You are a specialist.",
            SubagentRenderOptions {
                include_identity: true,
                include_safety_preamble: true,
                include_skills_catalog: true,
            },
            ToolCallFormat::Json,
        );

        assert!(rendered.contains("## Project Context"));
        assert!(rendered.contains("### SOUL.md"));
        assert!(rendered.contains("## Safety"));
        assert!(rendered.contains("## Available Skills"));
        assert!(rendered.contains("Parameters:"));
        assert!(rendered.contains("\"type\""));

        let native = render_subagent_system_prompt_with_format(
            &workspace,
            "reasoning-v1",
            &[0],
            &tools,
            "You are a specialist.",
            SubagentRenderOptions::narrow(),
            ToolCallFormat::Native,
        );
        assert!(native.contains("native tool-calling output"));
        assert!(!native.contains("## Safety"));

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
    fn dynamic_section_classification_matches_cache_boundary_rules() {
        assert!(is_dynamic_section("workspace"));
        assert!(is_dynamic_section("datetime"));
        assert!(is_dynamic_section("runtime"));
        assert!(!is_dynamic_section("tools"));
        assert!(!is_dynamic_section("identity"));
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
        };
        let rendered = UserMemorySection.build(&ctx).unwrap();
        assert!(rendered.contains("### user"));
        assert!(!rendered.contains("### empty"));
        assert_eq!(default_workspace_file_content("missing"), "");
    }
}
