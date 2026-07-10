//! [`SystemPromptBuilder`] — assembles ordered [`PromptSection`]s into a
//! final system-prompt string.

use super::sections::*;
use super::types::*;
use anyhow::Result;

/// Global style rules appended to every assembled system prompt, regardless
/// of which sections the agent opts in/out of. Kept tiny and byte-stable so
/// it doesn't bust the inference backend's prefix cache.
pub const GLOBAL_STYLE_SUFFIX: &str = "## Output style\n\n\
    - Do **not** use em-dashes (`—`). Replace them with commas, colons, \
    parentheses, or two short sentences. This applies to every output \
    you produce: chat replies, summaries, tool args, and file contents.\n\
    - **Be concise.** Lead with the answer, then only the detail the task \
    needs. No preamble, no recap of the request, no filler. Make the \
    response as long as the task requires and no longer: a one-line answer \
    for a simple ask, fuller treatment only when the task genuinely needs it.\n\
    - **Do not repeat yourself.** Don't restate facts, context, or results \
    already shown earlier in this conversation; reference them instead of \
    pasting them again.\n";

#[derive(Default)]
pub struct SystemPromptBuilder {
    pub(super) sections: Vec<Box<dyn PromptSection>>,
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
                //
                // The privileged `UserReflectionsSection` is appended
                // dynamically by `session::builder` when the
                // learning subsystem is enabled, alongside
                // `LearnedContextSection` / `UserProfileSection` — those
                // three are config-gated and intentionally not part of
                // the static default chain.
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

    /// Insert `section` immediately before the first existing section
    /// whose [`PromptSection::name`] matches `target_name`. When no
    /// matching section is present (most dynamic / sub-agent builders
    /// do not include `user_memory`, for example), the new section is
    /// appended at the end instead.
    ///
    /// Used by the session builder to guarantee that the privileged
    /// reflection block ranks ahead of broader memory sections like
    /// `user_memory`, even when the surrounding builder was assembled
    /// via [`Self::with_defaults`] which already contains them.
    pub fn insert_section_before(
        mut self,
        target_name: &str,
        section: Box<dyn PromptSection>,
    ) -> Self {
        let position = self.sections.iter().position(|s| s.name() == target_name);
        match position {
            Some(idx) => self.sections.insert(idx, section),
            None => self.sections.push(section),
        }
        self
    }

    /// Append a [`ToolMemoryRulesSection`] carrying a pre-fetched
    /// snapshot of Critical / High priority tool-scoped rules (#1400).
    ///
    /// Snapshot semantics — the rules are baked into the section at
    /// construction so the rendered system prompt stays byte-identical
    /// for the lifetime of the session. The session builder is
    /// responsible for pre-fetching via
    /// [`crate::openhuman::memory_tools::ToolMemoryStore::rules_for_prompt`]
    /// (or the `memory_tool_rules_for_prompt` RPC) before invoking
    /// this method.
    ///
    /// No-op when `rules` is empty.
    pub fn with_tool_memory_rules(
        mut self,
        rules: Vec<crate::openhuman::memory_tools::ToolMemoryRule>,
    ) -> Self {
        if rules.is_empty() {
            return self;
        }
        // Insert before the tool-catalogue section so these rules appear
        // adjacent to the tool listings and survive tail-biased trimming.
        // Falls back to push when no tools section is present.
        let section: Box<dyn PromptSection> = Box::new(
            crate::openhuman::memory_tools::ToolMemoryRulesSection::new(rules),
        );
        let tools_idx = self
            .sections
            .iter()
            .position(|s| s.name() == "tools" || s.name() == "tool_catalogue");
        match tools_idx {
            Some(idx) => self.sections.insert(idx, section),
            None => self.sections.push(section),
        }
        self
    }

    /// Append a "Memory context" section carrying the resolved chunks the
    /// subconscious LLM cited when it produced the reflection that
    /// spawned this thread (#623).
    ///
    /// Snapshot semantics — chunks are baked at construction so the
    /// rendered system prompt remains byte-identical for the lifetime of
    /// the session, preserving the inference backend's prefix cache hit.
    /// The session builder calls this when it detects a thread with a
    /// `subconscious_reflection`-origin seed message.
    ///
    /// No-op when `chunks` is empty.
    pub fn with_reflection_context(
        mut self,
        chunks: Vec<crate::openhuman::subconscious::SourceChunk>,
    ) -> Self {
        if chunks.is_empty() {
            return self;
        }
        self.sections
            .push(Box::new(ReflectionMemoryContextSection::new(chunks)));
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
        // Grounding / anti-hallucination contract is appended centrally here
        // (and in the narrow sub-agent renderer) rather than per-section, so
        // EVERY agent inherits the same anti-fabrication floor — including the
        // ~26 dynamic `agents/<id>/prompt.rs` builders that each hand-assemble
        // their own body via the `render_*` helpers and would otherwise have
        // to splice it in individually. Single source of truth: GROUNDING_BODY.
        // Placed near the tail (just before the output-style rules) so it reads
        // as a closing contract; byte-stable, so it stays cache-friendly.
        output.push_str(GROUNDING_BODY);
        output.push_str("\n\n");
        output.push_str(GLOBAL_STYLE_SUFFIX);
        output.push('\n');
        Ok(output)
    }
}
