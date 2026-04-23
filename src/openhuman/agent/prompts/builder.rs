use crate::openhuman::agent::prompts::types::*;
use anyhow::Result;

use super::sections::{
    IdentitySection, SafetySection, ToolsSection, UserFilesSection, UserMemorySection,
    WorkspaceSection,
};

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
                Box::new(UserMemorySection),
                Box::new(ToolsSection),
                Box::new(SafetySection),
                Box::new(WorkspaceSection),
                Box::new(super::sections::DateTimeSection),
                Box::new(super::sections::RuntimeSection),
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
