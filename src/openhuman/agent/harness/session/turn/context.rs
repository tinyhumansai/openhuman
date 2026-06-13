//! History, context, and system prompt management.

use super::super::turn_checkpoint::assistant_message_has_tool_calls;
use super::super::types::Agent;
use super::{collect_tree_root_summaries, sanitize_learned_entry};
use crate::openhuman::agent_tool_policy::render_tool_policy_boundary;
use crate::openhuman::context::prompt::{LearnedContextData, PromptContext, PromptTool};
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::tools::Tool;

use anyhow::Result;

impl Agent {
    // ─────────────────────────────────────────────────────────────────
    // History & prompt helpers
    // ─────────────────────────────────────────────────────────────────

    /// Truncates the conversation history to the configured maximum message count.
    ///
    /// System messages are always preserved. Older non-system messages are
    /// dropped first.
    pub(in super::super) fn trim_history(&mut self) {
        let max = self.config.max_history_messages;
        if self.history.len() <= max {
            return;
        }

        let mut system_messages = Vec::new();
        let mut other_messages = Vec::new();

        for msg in self.history.drain(..) {
            match &msg {
                ConversationMessage::Chat(chat) if chat.role == "system" => {
                    system_messages.push(msg);
                }
                _ => other_messages.push(msg),
            }
        }

        if other_messages.len() > max {
            let drop_count = other_messages.len() - max;
            other_messages.drain(0..drop_count);
        }

        // A cut that lands *between* an `AssistantToolCalls` and its
        // `ToolResults` leaves the window opening on an orphaned `ToolResults`.
        // Serialized, that is a `tool` message with no preceding `tool_calls`,
        // which the provider rejects with a 400 (the response streams back
        // empty and surfaces to the user as "Something went wrong"). Snap the
        // boundary forward past any leading orphaned results so the window
        // always starts on a clean turn (a `Chat` or an `AssistantToolCalls`).
        let orphan_lead = other_messages
            .iter()
            .take_while(|m| matches!(m, ConversationMessage::ToolResults(_)))
            .count();
        if orphan_lead > 0 {
            log::debug!(
                "[agent] trim_history snapped window past {orphan_lead} orphaned ToolResults \
                 (tool-cycle bisected by the {max}-message cap)"
            );
            other_messages.drain(0..orphan_lead);
        }

        self.history = system_messages;
        self.history.extend(other_messages);
    }

    /// Bound a resumed transcript prefix to the agent history window.
    ///
    /// Resume paths may load a long prior transcript directly into
    /// `cached_transcript_messages` (provider-ready `ChatMessage`s), which
    /// bypasses `self.history`-based trimming/reduction. Keep at most
    /// `max_history_messages` entries while preserving the leading system
    /// message when present.
    pub(in super::super) fn bound_cached_transcript_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Vec<ChatMessage> {
        let max = self.config.max_history_messages.max(1);
        if messages.len() <= max {
            return messages;
        }

        let has_system = matches!(messages.first(), Some(msg) if msg.role == "system");
        let keep_tail = if has_system {
            max.saturating_sub(1)
        } else {
            max
        };
        let start = messages.len().saturating_sub(keep_tail);

        // Same hazard as `trim_history`: the tail slice can open on a `tool`
        // message whose `tool_calls` opener fell outside the window, which the
        // provider rejects. Advance past any leading orphaned `tool` results so
        // the window starts on a clean turn.
        let tail = &messages[start..];
        let orphan_lead = tail.iter().take_while(|m| m.role == "tool").count();
        if orphan_lead > 0 {
            log::debug!(
                "[agent] bound_cached_transcript_messages snapped window past {orphan_lead} \
                 orphaned tool result(s) (tool-cycle bisected by the {max}-message cap)"
            );
        }
        let tail = &tail[orphan_lead..];

        let mut bounded = Vec::with_capacity(tail.len() + usize::from(has_system));
        if has_system {
            bounded.push(messages[0].clone());
        }
        bounded.extend(tail.iter().cloned());

        // TAURI-RUST-7: symmetric guard to the leading-orphan strip above. A
        // resumed transcript that ends on an `assistant` message containing
        // `tool_calls` (because the cached transcript was captured mid-cycle,
        // before the tool responses were persisted) is rejected by the
        // provider with `400 An assistant message with 'tool_calls' must be
        // followed by tool messages`. Pop any such trailing assistant
        // tool_calls so the bounded transcript ends on a clean turn boundary.
        let mut dropped_tail = 0usize;
        while bounded
            .last()
            .map(assistant_message_has_tool_calls)
            .unwrap_or(false)
        {
            bounded.pop();
            dropped_tail += 1;
        }
        if dropped_tail > 0 {
            log::debug!(
                "[agent] bound_cached_transcript_messages stripped {dropped_tail} trailing \
                 assistant tool_calls message(s) without paired tool responses"
            );
        }

        bounded
    }

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
            let general = crate::openhuman::memory::preferences::load_general_preferences(
                &self.memory,
                crate::openhuman::memory::preferences::STANDING_PREFS_LIMIT,
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
        let general = crate::openhuman::memory::preferences::load_general_preferences(
            &self.memory,
            crate::openhuman::memory::preferences::STANDING_PREFS_LIMIT,
        )
        .await;

        // Explicit user reflections — privileged memory class. Pulled
        // separately from observations/patterns so the prompt assembly
        // can render them ahead of generic tree summaries.
        let reflection_entries = self
            .memory
            .list(
                Some(crate::openhuman::learning::reflection::REFLECTIONS_NAMESPACE),
                Some(&MemoryCategory::Custom(
                    crate::openhuman::learning::reflection::REFLECTIONS_NAMESPACE.into(),
                )),
                None,
            )
            .await
            .unwrap_or_default();

        // Pull every namespace's root-level summary from the tree
        // summarizer. This is the densest user memory we can hand the
        // orchestrator: each root holds up to 20 000 tokens of distilled
        // long-term context. Done synchronously here because the calls
        // are filesystem reads, not provider/network round-trips, and
        // happen exactly once per session (only on the first turn).
        //
        // Per-namespace + total caps come from the user-facing memory
        // window preset on `AgentConfig` so changing the slider in the
        // UI takes effect on the very next session-start.
        let limits = self.config.resolved_memory_limits();
        let tree_root_summaries = collect_tree_root_summaries(
            &self.workspace_dir,
            limits.per_namespace_max_chars,
            limits.total_tree_max_chars,
        );

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
        let tools_slice: &[Box<dyn Tool>] = self.tools.as_slice();
        let instructions = self
            .tool_dispatcher
            .prompt_instructions_for_specs(self.visible_tool_specs.as_slice())
            .unwrap_or_else(|| self.tool_dispatcher.prompt_instructions(tools_slice));
        // Adapt the owned Box<dyn Tool> slice into the shared PromptTool
        // shape that every prompt-building call-site uses. Temporary vec
        // borrows from `tools_slice` and lives for the duration of the
        // prompt build.
        let prompt_tools = PromptTool::from_tools(tools_slice);
        let prompt_visible_tool_names = self.tool_policy_session.visible_tool_names_for_prompt();
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            agent_id: &self.agent_definition_name,
            tools: &prompt_tools,
            workflows: &self.workflows,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &prompt_visible_tool_names,
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            connected_integrations: &self.connected_integrations,
            connected_identities_md: crate::openhuman::agent::prompts::render_connected_identities(
            ),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::openhuman::app_state::peek_cached_current_user_identity(),
            // TODO(phase-2): Wire personality context into the live agent turn.
            // Currently personalities only take effect during delegate_to_personality sub-agent runs.
            // To activate: load the active profile via AgentProfileStore::resolve(), build
            // PersonalityContext::from_profile(), and populate these fields.
            personality_soul_md: None, // TODO: personality_ctx.soul_md_override
            personality_memory_md: None, // TODO: personality_ctx.memory_md_override
            personality_roster: vec![], // TODO: build_personality_roster(&workspace_dir)
        };
        // Route through the global context manager so every
        // prompt-building call-site — main agent, sub-agent runner,
        // channel runtimes — shares one builder configuration.
        let mut prompt = self.context.build_system_prompt(&ctx)?;
        if let Some(boundary) = render_tool_policy_boundary(&self.tool_policy_session, 2048) {
            prompt = format!("{boundary}\n\n{prompt}");
        }
        Ok(prompt)
    }
}
