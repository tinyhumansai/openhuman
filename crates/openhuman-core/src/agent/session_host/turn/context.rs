//! History, context, and system prompt management.

use super::super::types::OpenHumanSessionHost;
use crate::agent::prompts::{
    LearnedContextData, PromptContext, PromptTool, tool_call_format_from_dialect,
};
use crate::memory::MemoryCategory;
use crate::tools::agent_policy::render_tool_policy_boundary;

use anyhow::Result;

async fn collect_tree_root_summaries(
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<crate::agent::prompts::NamespaceSummary> {
    use crate::memory::api::provider::MemoryProvider;

    let Ok(guard) = crate::memory::ops::guard::active_memory_guard().await else {
        return Vec::new();
    };
    let Some(tree) = guard.as_tree() else {
        return Vec::new();
    };
    match tree
        .root_summaries_with_caps(per_namespace_cap, total_cap)
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|row| crate::agent::prompts::NamespaceSummary {
                namespace: row.namespace,
                body: row.body,
                updated_at: row.updated_at,
            })
            .collect(),
        Err(error) => {
            log::warn!("[session-runtime] tree root summaries unavailable: {error}");
            Vec::new()
        }
    }
}

fn sanitize_learned_entry(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let sanitized: String = trimmed.chars().take(200).collect();
    if sanitized.contains("Bearer ")
        || sanitized.contains("sk-")
        || sanitized.contains("ghp_")
        || sanitized.contains("-----BEGIN")
    {
        return "[redacted: potential secret]".to_string();
    }
    sanitized
}

impl OpenHumanSessionHost {
    /// Pre-fetches learned context data from memory (observations, patterns, user profile).
    ///
    /// This is an async, non-blocking operation that populates the context
    /// for the system prompt.
    ///
    /// # Explicit-preferences narrow path
    ///
    /// When `learning_enabled` is `false` but `explicit_preferences_enabled`
    /// is `true`, only the `user_profile` namespace (pinned preferences from
    /// the `remember_preference` tool) is fetched and returned.  All other
    /// inference-derived data (observations, patterns, reflections, tree
    /// summaries) remains empty — the inference stack is not touched.
    pub(in super::super) async fn fetch_learned_context(&self) -> LearnedContextData {
        // Fast path: neither the full learning subsystem nor the explicit
        // preferences path is active — skip all memory reads.
        if !self.learning_enabled && !self.explicit_preferences_enabled {
            tracing::debug!(
                "[learning] fetch_learned_context: both learning_enabled and \
                 explicit_preferences_enabled are false — returning empty context"
            );
            return LearnedContextData::default();
        }

        // Narrow explicit-preferences path (Lane A): inject the latest-N general
        // (always-on) preferences written via `save_preference`. Topic-scoped
        // (situational) prefs are NOT injected here — they ride the user message
        // via per-turn recall (Lane B). The legacy `user_profile` pinned namespace
        // is no longer read here; explicit prefs now live in `user_pref_general`.
        if !self.learning_enabled && self.explicit_preferences_enabled {
            let general = crate::memory::preferences::load_general_preferences_on(
                &self.memory,
                crate::memory::preferences::STANDING_PREFS_LIMIT,
            )
            .await;
            tracing::debug!(
                "[learning] fetch_learned_context: explicit_preferences_enabled — loaded {} general preference(s) for the system prompt",
                general.len()
            );
            return LearnedContextData {
                user_profile: general,
                ..LearnedContextData::default()
            };
        }

        // Full learning path: fetch all inference-derived data.
        tracing::debug!(
            "[learning] fetch_learned_context: learning_enabled=true — fetching full context"
        );

        let obs_entries = self
            .memory
            .list(
                Some("learning_observations"),
                Some(&MemoryCategory::Custom("learning_observations".into())),
                None,
            )
            .await
            .unwrap_or_default();

        let pat_entries = self
            .memory
            .list(
                Some("learning_patterns"),
                Some(&MemoryCategory::Custom("learning_patterns".into())),
                None,
            )
            .await
            .unwrap_or_default();

        // Standing preferences come from the explicit two-lane store (Lane A),
        // not the inferred `user_profile` facets — those are demoted: no longer
        // injected as ground truth. A high-confidence inferred facet should be
        // *proposed* to the user (and pinned via `save_preference` on
        // confirmation), not silently treated as a standing preference.
        let general = crate::memory::preferences::load_general_preferences_on(
            &self.memory,
            crate::memory::preferences::STANDING_PREFS_LIMIT,
        )
        .await;

        // Explicit user reflections — privileged memory class. Pulled
        // separately from observations/patterns so the prompt assembly
        // can render them ahead of generic tree summaries.
        let reflection_entries = self
            .memory
            .list(
                Some(crate::agent::learning::reflection::REFLECTIONS_NAMESPACE),
                Some(&MemoryCategory::Custom(
                    crate::agent::learning::reflection::REFLECTIONS_NAMESPACE.into(),
                )),
                None,
            )
            .await
            .unwrap_or_default();

        // Pull every namespace's root-level summary from the tree
        // summarizer. This is the densest user memory we can hand the
        // orchestrator: each root holds up to 20 000 tokens of distilled
        // long-term context. Awaited inline, alongside the four memory reads
        // above: the shared tree's roots come from the bound driver now
        // (#5560) rather than from a host-side filesystem scan, and this
        // happens exactly once per session (only on the first turn).
        //
        // Per-namespace + total caps come from the user-facing memory
        // window preset on `AgentConfig` so changing the slider in the
        // UI takes effect on the very next session-start.
        let limits = self.config.resolved_memory_limits();
        let tree_root_summaries = collect_tree_root_summaries(
            limits.per_namespace_max_chars,
            limits.total_tree_max_chars,
        )
        .await;

        LearnedContextData {
            observations: obs_entries
                .iter()
                .rev()
                .take(5)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            patterns: pat_entries
                .iter()
                .take(3)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            user_profile: general,
            // Cap reflections at 10 to keep the privileged section
            // bounded — the issue requires reflections improve context
            // rather than flood it. Newest first.
            reflections: reflection_entries
                .iter()
                .rev()
                .take(10)
                .map(|e| sanitize_learned_entry(&e.content))
                .collect(),
            tree_root_summaries,
        }
    }

    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    pub fn build_system_prompt(&self, learned: LearnedContextData) -> Result<String> {
        // `visible_tool_specs` holds shared `Arc<ToolSpec>` leaves. Materialise
        // the canonical dialect input for this prompt build.
        let visible_specs_owned: Vec<tinytools::ToolSpec> = self
            .visible_tool_specs
            .iter()
            .map(|spec| spec.as_ref().clone())
            .collect();
        let instructions = self
            .tool_dispatcher
            .prompt_instructions(&visible_specs_owned);
        // Adapt the agent's whole callable surface into the shared PromptTool
        // shape that every prompt-building call-site uses. Temporary vec
        // borrows from the two tool `Arc`s and lives for the duration of the
        // prompt build. The synthesised delegates belong here: the catalogue
        // this renders is what tells the model a `delegate_*` tool exists.
        let all_tools = self.all_tool_refs();
        let prompt_tools = PromptTool::from_tool_refs(all_tools.iter().copied());
        let prompt_visible_tool_names = self.tool_policy_session.visible_tool_names_for_prompt();
        // Load AGENTS.md instruction layers once per system-prompt build (never
        // re-read per turn — the caller builds the prompt once at session start
        // and reuses the bytes, preserving the frozen-prefix / KV-cache
        // contract). Global layer from the workspace dir; project layer from the
        // effective action dir. Gated by `agents_md_enabled`.
        let agents_md = if self.config.agents_md_enabled {
            crate::agent::prompts::load_agents_md_layers(&self.workspace_dir, &self.action_dir)
        } else {
            tracing::debug!(
                "[agents_md] disabled by config; skipping AGENTS.md injection for main agent"
            );
            crate::agent::prompts::AgentsMdContent::default()
        };
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            agent_id: &self.agent_definition_name,
            tools: &prompt_tools,
            workflows: &self.workflows,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &prompt_visible_tool_names,
            tool_call_format: tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            connected_integrations: &self.connected_integrations,
            connected_identities_md: crate::agent::prompts::render_connected_identities(),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![], // TODO: build_personality_roster(&workspace_dir)
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        // Route through the global context manager so every
        // prompt-building call-site — main agent, sub-agent runner,
        // channel runtimes — shares one builder configuration.
        let prompt = self
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .build_system_prompt(&ctx)?;
        // Appended, not prepended (#5704). Every line of this block is
        // session-scoped — agent id, channel, entry point, risk level, the
        // allowed-tool list — so putting it first moves the prompt's first
        // diverging byte to offset 0 and costs the inference backend's
        // automatic prefix cache everything behind it. That is the same
        // concern that keeps DateTimeSection out of `for_subagent` and keeps
        // the connected-server overview sorted. The model reads the whole
        // system message either way.
        //
        // It also keeps the archetype/persona as the prompt's opening line,
        // which the prepend had replaced with a constant heading for every
        // agent.
        let boundary = render_tool_policy_boundary(&self.tool_policy_session, 2048);
        Ok(append_tool_policy_boundary(prompt, boundary))
    }
}

/// Place the tool-policy boundary block relative to the assembled prompt.
///
/// Separated from [`OpenHumanSessionHost`] so the ordering can be tested without standing up a
/// session: everything that decides the placement is in these two arguments.
fn append_tool_policy_boundary(prompt: String, boundary: Option<String>) -> String {
    match boundary {
        Some(boundary) => format!("{prompt}\n\n{boundary}"),
        None => prompt,
    }
}
