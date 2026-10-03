//! [`SystemPromptBuilder`] — assembles ordered [`PromptSection`]s into a
//! final system-prompt string.

/// A rendered system prompt together with the byte offsets at which a provider
/// may place a prompt-cache breakpoint.
///
/// Offsets are ends-of-tier, in ascending order, and always fall on a UTF-8
/// character boundary because they are taken at a point where only whole
/// sections have been pushed. At most two are produced today (end of `Stable`,
/// end of `Context`), comfortably inside the four Anthropic accepts.
#[derive(Debug, Clone, Default)]
pub struct TieredPrompt {
    /// The assembled prompt — byte-identical to what [`SystemPromptBuilder::build`] returns.
    pub text: String,
    /// Ascending byte offsets into [`Self::text`].
    pub breakpoints: Vec<usize>,
    /// The bytes of each tier, in tier order, with empty tiers omitted.
    ///
    /// Concatenating the strings in order reproduces [`Self::text`]. A host
    /// that wants the provider to see the tiers as separate cacheable
    /// segments sends one system message per part (`runtime_session.rs`).
    pub parts: Vec<(PromptTier, String)>,
}

impl TieredPrompt {
    /// The tiers as separate strings, ready to become one system message each.
    ///
    /// `Stable` and `Context` are merged into the first message: both are
    /// fixed for the whole session, and one fewer message is one fewer thing a
    /// provider can reject. `Volatile` (when present) is the second message,
    /// so a newly connected service changes the
    /// second segment and leaves the first byte-identical.
    #[must_use]
    pub fn system_messages(&self) -> Vec<String> {
        let mut head = String::new();
        let mut tail = String::new();
        for (tier, part) in &self.parts {
            match tier {
                PromptTier::Stable | PromptTier::Context => head.push_str(part),
                PromptTier::Volatile => tail.push_str(part),
            }
        }
        let mut messages = Vec::with_capacity(2);
        if !head.trim().is_empty() {
            messages.push(head.trim_end().to_string());
        }
        if !tail.trim().is_empty() {
            messages.push(tail.trim_end().to_string());
        }
        messages
    }
}

use super::render_helpers::sync_workspace_file;
use super::sections::*;
use super::types::*;
use anyhow::Result;
use std::path::Path;

/// Global style rules appended to every assembled system prompt, regardless
/// of which sections the agent opts in/out of. Kept tiny and byte-stable so
/// it doesn't bust the inference backend's prefix cache.
///
/// These are the rules that make output read as written by a person. There
/// used to be a **Be concise** bullet here too ("lead with the answer, then
/// only the detail the task needs ... a one-line answer for a simple ask").
/// It was removed deliberately: brevity is not the same goal as sounding
/// human, and a global length ceiling was truncating answers that had more to
/// say. Lead-with-the-answer and no-preamble survive in the per-agent voice
/// sections, where they can be phrased as ordering rather than as a budget.
/// Do not reintroduce a global length rule here.
///
/// The text itself now lives in `STYLE.md` (#5701) rather than in this
/// constant, so it can be tuned on disk without a rebuild. This value is the
/// bundled seed and the fallback when the workspace copy cannot be read; the
/// authoritative content is whatever `sync_workspace_file` last wrote, plus
/// any user edit on top of it.
pub const GLOBAL_STYLE_SUFFIX: &str = include_str!("STYLE.md");

/// The writing-style block appended to every agent's prompt.
///
/// Reads the workspace `STYLE.md`, seeding it from the bundled copy first so a
/// fresh workspace still gets the rules. Falls back to the bundled text if the
/// file cannot be read, because a prompt with no style contract at all is a
/// worse failure than a stale one.
///
/// Synced here rather than only in [`IdentitySection`] because agents that set
/// `omit_identity` skip that section entirely, and they need the style rules
/// too.
fn global_style_block(workspace_dir: &Path) -> String {
    sync_workspace_file(workspace_dir, "STYLE.md");
    std::fs::read_to_string(workspace_dir.join("STYLE.md")).unwrap_or_else(|error| {
        tracing::warn!(
            "[style] could not read workspace STYLE.md ({error}); \
             falling back to the bundled copy"
        );
        GLOBAL_STYLE_SUFFIX.to_string()
    })
}

#[derive(Default)]
pub struct SystemPromptBuilder {
    pub(super) sections: Vec<Box<dyn PromptSection>>,
}

impl SystemPromptBuilder {
    pub fn with_defaults() -> Self {
        Self {
            sections: vec![
                Box::new(IdentitySection),
                // Project instructions (AGENTS.md) sit right after the identity
                // bootstrap and before the tool catalogue — standing, per-project
                // guidance the model should read alongside identity. Both
                // layers are pre-loaded into `PromptContext` and this section is
                // empty (skipped) when neither exists or the gate is off.
                Box::new(AgentsInstructionsSection),
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
    /// the `omit_*` flags from the [`crate::agent::harness::definition::AgentDefinition`]:
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
    ) -> Self {
        let mut sections: Vec<Box<dyn PromptSection>> =
            vec![Box::new(ArchetypePromptSection::new(archetype_prompt_text))];

        if !omit_identity {
            sections.push(Box::new(IdentitySection));
        }
        // Project instructions (AGENTS.md) — same placement as the default
        // chain (after identity, before tools). Empty (skipped) unless the
        // caller pre-loaded content onto `PromptContext`.
        sections.push(Box::new(AgentsInstructionsSection));
        // Tools section is always included — the sub-agent needs to see
        // its own (filtered) tool catalogue.
        sections.push(Box::new(ToolsSection));
        if !omit_safety_preamble {
            sections.push(Box::new(SafetySection));
        }
        // Skills catalogue and connected integrations are rendered by
        // the individual agent's `prompt.rs` when that agent needs
        // them (orchestrator/welcome for the delegator voice). The shared
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
    pub fn from_dynamic(builder: crate::agent::harness::definition::PromptBuilder) -> Self {
        Self {
            sections: vec![
                Box::new(DynamicPromptSection::new(builder)),
                // Project instructions (AGENTS.md). The ~26 dynamic
                // `agents/<id>/prompt.rs` builders (orchestrator / main chat,
                // welcome, …) hand-assemble their own body
                // via the `render_*` helpers and none of them individually call
                // `render_agents_md`, so the pre-loaded AGENTS.md layers on
                // `PromptContext` would otherwise be silently dropped for the
                // primary agent. Inject the shared section centrally here —
                // mirroring how `build()` appends the grounding contract for all
                // dynamic builders — so every dynamic agent inherits the same
                // AGENTS.md injection as the `with_defaults` / `for_subagent`
                // chains. Rendered after the agent's own body (as trailing
                // standing guidance) and before the central grounding suffix.
                // Empty (skipped) when neither layer carries content or the
                // `agents_md_enabled` gate is off.
                Box::new(AgentsInstructionsSection),
            ],
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
        Ok(self.build_tiered(ctx)?.text)
    }

    /// Assemble the prompt **and** report where its cache tiers end.
    ///
    /// Sections are emitted grouped by [`PromptSection::tier`] — every
    /// `Stable` section in declaration order, then every `Context` one, then
    /// every `Volatile` one. Within a tier the declaration order is preserved
    /// exactly, so this is a stable partition rather than a sort: a section
    /// that does not change tier does not change its neighbours.
    ///
    /// The grouping is the whole point. A prefix is reusable only up to the
    /// first byte that differs, so a volatile section emitted early throws away
    /// every stable byte behind it, so volatile sections (connected services, the
    /// signed-in user) always render after the tool catalogue, the safety
    /// contract and the writing-style rules regardless of declaration order.
    ///
    /// It does not change the prompt's **size**: the same sections render the
    /// same bytes, in a different order (`scripts/prompt-report.sh` shows the
    /// per-agent totals).
    pub fn build_tiered(&self, ctx: &PromptContext<'_>) -> Result<TieredPrompt> {
        // Render each section once and bucket its parts by tier. A section
        // usually yields one part in its own tier; a dynamic builder that
        // marks its tiers yields several (see `PromptSection::build_parts`).
        let mut buckets: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut has_grounding = false;
        for section in &self.sections {
            for (tier, part) in section.build_parts(ctx)? {
                if part.trim().is_empty() {
                    continue;
                }
                if part.contains(GROUNDING_HEADING) {
                    has_grounding = true;
                }
                buckets[tier_index(tier)].push(part.trim_end().to_string());
            }
        }
        // The grounding contract and the writing-style rules are byte-stable
        // and shared by every agent, so they close the *stable* tier: behind
        // the identity and rules, ahead of anything that changes per session.
        // Grounding is skipped when the agent's own prompt already carries
        // the contract under the shared heading (the orchestrator does), so
        // it never ships twice.
        if !has_grounding {
            buckets[tier_index(PromptTier::Stable)].push(GROUNDING_BODY.trim_end().to_string());
        }
        buckets[tier_index(PromptTier::Stable)]
            .push(global_style_block(ctx.workspace_dir).trim_end().to_string());

        let mut text = String::new();
        let mut breakpoints: Vec<usize> = Vec::new();
        let mut parts: Vec<(PromptTier, String)> = Vec::new();
        for tier in [
            PromptTier::Stable,
            PromptTier::Context,
            PromptTier::Volatile,
        ] {
            let bucket = &buckets[tier_index(tier)];
            if bucket.is_empty() {
                continue;
            }
            let mut rendered = String::new();
            for part in bucket {
                rendered.push_str(part);
                rendered.push_str("\n\n");
            }
            text.push_str(&rendered);
            parts.push((tier, rendered));
            // A boundary is only worth declaring when something can still
            // follow it; one at the very end is the provider's default anyway.
            if tier != PromptTier::Volatile {
                breakpoints.push(text.len());
            }
        }
        // Drop a trailing breakpoint that coincides with the end of the text
        // (the prompt ended on a non-volatile tier).
        if breakpoints.last() == Some(&text.len()) {
            breakpoints.pop();
        }
        let text = format!("{}\n", text.trim_end());
        Ok(TieredPrompt {
            text,
            breakpoints,
            parts,
        })
    }
}

fn tier_index(tier: PromptTier) -> usize {
    match tier {
        PromptTier::Stable => 0,
        PromptTier::Context => 1,
        PromptTier::Volatile => 2,
    }
}
