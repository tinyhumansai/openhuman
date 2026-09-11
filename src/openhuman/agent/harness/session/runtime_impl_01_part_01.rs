
impl Agent {

    const EVENT_ERROR_MAX_CHARS: usize = 256;

    // ─────────────────────────────────────────────────────────────────
    // Small accessors used by `run_single` + `turn` + sub-agent runner
    // ─────────────────────────────────────────────────────────────────

    pub(super) fn event_session_id(&self) -> &str {
        &self.event_session_id
    }

    pub(super) fn event_channel(&self) -> &str {
        &self.event_channel
    }

    /// The agent definition id this session is running
    /// (`"welcome"`, `"orchestrator"`, `"integrations_agent"`, …).
    ///
    /// Exposed so callers that build sessions via
    /// [`Agent::from_config_for_agent`] can stamp the resolved id onto
    /// correlation logs and progress events without reaching for the
    /// source `Config`. See [`AgentBuilder::agent_definition_name`]
    /// for the full list of downstream surfaces (transcript filename,
    /// transcript metadata header, and `PromptContext::agent_id`) that
    /// read this field.
    pub fn agent_definition_name(&self) -> &str {
        &self.agent_definition_name
    }

    /// Returns a new `AgentBuilder`.
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    /// Clone the agent's model source. Used by the sub-agent runner /
    /// parent-context builder to share the parent's provider instance with
    /// spawned sub-agents (so they share connection pools, retry budgets, and
    /// rate-limit state) — issue #4249, Phase 3 / Motion A.
    pub fn turn_model_source(&self) -> crate::openhuman::agent::tinyagents::TurnModelSource {
        self.turn_model_source.clone()
    }

    /// Borrow the agent's durable tool registry as a slice. Used by the
    /// sub-agent runner to filter the parent's tool registry per-archetype.
    ///
    /// This is **not** the agent's whole callable surface — the synthesised
    /// delegation tools live in their own `Arc`. Use [`Self::all_tool_refs`]
    /// when you mean "every tool this agent can run".
    pub fn tools(&self) -> &[Box<dyn Tool>] {
        self.tools.as_slice()
    }

    /// Clone the agent's durable tools `Arc` for sharing with sub-agents.
    ///
    /// Deliberately excludes [`Self::synthesized_tools_arc`]: a sub-agent must
    /// never be handed a `delegate_*` tool, which the harness re-asserts at
    /// registration time (issue #4452).
    pub fn tools_arc(&self) -> Arc<Vec<Box<dyn Tool>>> {
        Arc::clone(&self.tools)
    }

    /// Clone the agent's synthesised delegation tools `Arc`.
    ///
    /// Replaced wholesale on every [`Agent::refresh_delegation_tools`], so a
    /// clone taken here is a stable snapshot for the rest of the caller's turn
    /// even if the connection set changes underneath it.
    pub fn synthesized_tools_arc(&self) -> Arc<Vec<Box<dyn Tool>>> {
        Arc::clone(&self.synthesized_tools)
    }

    /// Every tool this agent can execute: the durable registry first, then the
    /// synthesised delegation set — the same order as [`Self::tool_specs`] and
    /// turn dispatch.
    ///
    /// Borrowed rather than materialised as a `Vec<Box<dyn Tool>>` because
    /// `Box<dyn Tool>` is not cloneable — the two sets can be read together but
    /// never merged into one owned slice.
    pub fn all_tool_refs(&self) -> Vec<&dyn Tool> {
        self.tools
            .iter()
            .chain(self.synthesized_tools.iter())
            .map(|t| t.as_ref())
            .collect()
    }

    /// Borrow the agent's tool specs (pre-serialised). Captured at
    /// turn-start so sub-agents can pass byte-identical schemas to the
    /// provider for prefix-cache reuse.
    pub fn tool_specs(&self) -> &[Arc<ToolSpec>] {
        self.tool_specs.as_slice()
    }

    /// Clone the agent's full tool specs `Arc` (durable and synthesised).
    pub fn tool_specs_arc(&self) -> Arc<Vec<Arc<ToolSpec>>> {
        Arc::clone(&self.tool_specs)
    }

    /// Clone the agent's provider-facing spec list: visible, policy-allowed,
    /// de-duplicated, synthesised delegates included.
    pub fn visible_tool_specs_arc(&self) -> Arc<Vec<Arc<ToolSpec>>> {
        Arc::clone(&self.visible_tool_specs)
    }

    /// Clone the specs of the durable registry alone, index for index with
    /// [`Self::tools_arc`] — the pair a sub-agent is handed, so a child never
    /// sees a spec for a synthesised delegate it holds no instance for.
    pub fn durable_tool_specs_arc(&self) -> Arc<Vec<Arc<ToolSpec>>> {
        Arc::clone(&self.durable_tool_specs)
    }

    #[cfg(test)]
    pub(crate) fn visible_tool_names_for_test(&self) -> &std::collections::HashSet<String> {
        &self.visible_tool_names
    }

    #[cfg(test)]
    pub(crate) fn subagent_tool_ceiling_names_for_test(
        &self,
    ) -> &std::collections::HashSet<String> {
        &self.subagent_tool_ceiling_names
    }

    /// Borrow the agent's memory backing store as an `Arc`.
    pub fn memory_arc(&self) -> Arc<dyn Memory> {
        Arc::clone(&self.memory)
    }

    /// The full host [`Config`](crate::openhuman::config::Config) this session
    /// was built with, when it was built through the factory.
    ///
    /// `None` on the bare-builder path (`AgentBuilder` without
    /// `AgentFactory`), which is used by tests and by callers assembling a
    /// session by hand. Every capability adapter that needs host config treats
    /// `None` as "not available" rather than loading one itself — see
    /// [`Self::host_capabilities_available`].
    pub fn runtime_config(&self) -> Option<Arc<crate::openhuman::config::Config>> {
        self.runtime_config.clone()
    }

    /// Whether the config-dependent capability adapters can be built from this
    /// session.
    ///
    /// Four of the ten host capabilities (`BudgetGate`, `ContextComposer`,
    /// `ModelResolver`, and the policy half of `SecurityGate`) need a full
    /// `Config`, which only the factory path supplies. This is the one-line
    /// check a caller uses before reaching for them, so "this session cannot
    /// answer that" stays distinguishable from "the capability failed" — the
    /// same absence-versus-failure rule the traits themselves are built on.
    pub fn host_capabilities_available(&self) -> bool {
        self.runtime_config.is_some()
    }

    /// OpenHuman's [`AgentMemory`](tinyagents_harness::host::AgentMemory)
    /// capability over this session's memory backend.
    ///
    /// Built on demand rather than stored: it is a thin adapter over an `Arc`
    /// the session already holds, so constructing one is a refcount bump, and
    /// storing it would create a second handle that could drift from
    /// `self.memory` if the backend were ever swapped.
    pub fn host_agent_memory(
        &self,
    ) -> crate::openhuman::agent::tinyagents::host::OpenHumanAgentMemory {
        crate::openhuman::agent::tinyagents::host::OpenHumanAgentMemory::new(self.memory_arc())
    }

    /// OpenHuman's [`ExperienceStore`](tinyagents_harness::host::ExperienceStore)
    /// capability, scoped to this session's agent profile.
    ///
    /// Writes go to this session's own `memory`; recall additionally consults
    /// `shared_experience_memory` when the session was given one.
    ///
    /// That asymmetry mirrors the live turn path in `session/turn/core.rs`. For
    /// a dedicated-profile session `memory` is the profile-local store and
    /// `shared_experience_memory` is the global one holding unstamped records
    /// from pre-profile builds — so reading both is what keeps old experience
    /// reachable, while writing only to the profile-local store is what keeps
    /// new records inside the profile subtree.
    pub fn host_experience_store(
        &self,
    ) -> crate::openhuman::agent::tinyagents::host::OpenHumanExperienceStore {
        crate::openhuman::agent::tinyagents::host::OpenHumanExperienceStore::with_profile(
            self.memory_arc(),
            self.active_profile_id.clone(),
        )
        .with_shared_recall_memory(self.shared_experience_memory.clone())
    }

    /// The agent's working directory.
    pub fn workspace_dir(&self) -> &std::path::Path {
        &self.workspace_dir
    }

    /// The agent's currently-configured model name (before per-turn
    /// auto-classification).
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Override the base model this session runs its top-level turns on. Set
    /// once before running: per-turn classification is disabled (the main agent
    /// is pinned to its configured model for KV-cache stability — see the model
    /// pin in `turn/core.rs`), so this sticks for the session and is not flipped
    /// mid-conversation. The realtime voice harness uses it to pin a fast,
    /// non-thinking model within the provider's response-time ceiling.
    pub fn set_model_name(&mut self, model_name: impl Into<String>) {
        self.model_name = model_name.into();
    }

    /// The agent's currently-configured temperature.
    pub fn temperature(&self) -> f64 {
        self.temperature
    }

    /// The agent's loaded workflows, if any.
    pub fn workflows(&self) -> &[crate::openhuman::skills::Workflow] {
        &self.workflows
    }

    /// Active Composio integrations fetched at session start.
    pub fn connected_integrations(
        &self,
    ) -> &[crate::openhuman::agent::context::prompt::ConnectedIntegration] {
        &self.connected_integrations
    }

    /// This session's transcript key — `"{unix_ts}_{agent_id}"`,
    /// generated once at build time. Sub-agents chain this into their
    /// own transcript filenames so the parent → child hierarchy is
    /// visible on disk.
    pub fn session_key(&self) -> &str {
        &self.session_key
    }

    /// The ancestor chain of session keys for a sub-agent, joined with
    /// `__`. `None` for a root session. Root + prefix together produce
    /// the full transcript stem.
    pub fn session_parent_prefix(&self) -> Option<&str> {
        self.session_parent_prefix.as_deref()
    }

    /// Replace the agent's connected integrations (e.g. from a cached
    /// fetch result when the agent was built outside the normal turn loop).
    pub fn set_connected_integrations(
        &mut self,
        integrations: Vec<crate::openhuman::agent::context::prompt::ConnectedIntegration>,
    ) {
        self.connected_integrations = integrations;
        self.connected_integrations_initialized = true;
        self.last_seen_integrations_hash =
            crate::openhuman::integrations::composio::connected_set_hash(
                &self.connected_integrations,
            );
    }

    /// The agent's runtime config snapshot.
    pub fn agent_config(&self) -> &crate::openhuman::config::AgentConfig {
        &self.config
    }

    /// Override the agent's tool-iteration cap after construction.
    ///
    /// Issue #4868 — `build_session_agent_inner` now stamps every agent with
    /// its `AgentDefinition::effective_max_iterations()`, which is the correct
    /// behavior for direct-invocation call sites. A handful of callers need a
    /// *different* cap than the definition's declared budget (e.g. long-running
    /// workflow/task-dispatcher runs that intentionally exceed any single
    /// agent's normal budget). Those callers should apply their override
    /// AFTER construction via this setter, so the shared definition-cap logic
    /// in the builder doesn't get silently clobbered by pre-construction
    /// mutations (and vice versa).
    pub fn set_max_tool_iterations(&mut self, cap: usize) {
        self.config.max_tool_iterations = cap;
    }

    /// Returns the current conversation history.
    pub fn history(&self) -> &[ConversationMessage] {
        &self.history
    }

    pub fn set_event_context(&mut self, session_id: impl Into<String>, channel: impl Into<String>) {
        self.event_session_id = session_id.into();
        self.event_channel = channel.into();
        self.rebuild_tool_policy_session();
    }

    /// Override the agent definition name used for session transcript
    /// file paths. Callers (e.g. the web channel) use this to scope
    /// transcripts per thread so each conversation thread gets its own
    /// transcript namespace instead of sharing one by agent type.
    ///
    /// Also rebuilds [`Self::session_key`] so the next call to
    /// `persist_session_transcript` writes to a path keyed by the new
    /// name. Without this, persist would keep using the builder-time
    /// name (e.g. `"orchestrator"`) while
    /// `find_latest_transcript` searches for the post-rename name (e.g.
    /// `"orchestrator_thread-6ad6d"`), and resume on cold boot would
    /// silently miss every prior transcript — the LLM would then run
    /// each new turn with no conversation history.
    pub fn set_agent_definition_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        let sanitized: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // Preserve the original unix-timestamp prefix from the builder
        // so sub-agent spawn collisions remain impossible. Falls back
        // to "0" if the existing key is in an unexpected shape.
        let prefix = self
            .session_key
            .split_once('_')
            .map(|(p, _)| p)
            .filter(|p| !p.is_empty())
            .unwrap_or("0");
        self.session_key = format!("{prefix}_{sanitized}");
        self.agent_definition_name = name;
        self.rebuild_tool_policy_session();
    }

    /// Attach a progress event sender for real-time turn updates.
    ///
    /// When set, the turn loop emits [`AgentProgress`] events so
    /// callers (e.g. the web channel) can surface live tool-call and
    /// iteration updates to the UI. Pass `None` to disable.
    pub fn set_on_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::openhuman::agent::progress::AgentProgress>>,
    ) {
        self.on_progress = tx;
    }

    /// Bind this session's acting tools (shell / file / git) to `descriptor`'s
    /// root as their default working directory.
    ///
    /// The post-build counterpart of
    /// [`AgentBuilder::workspace_descriptor`](crate::openhuman::agent::AgentBuilder::workspace_descriptor),
    /// for callers that construct the agent through
    /// [`Agent::from_config`](crate::openhuman::agent::Agent::from_config) and
    /// therefore never see the builder — notably the per-turn `cwd` of
    /// [`agent_chat`](crate::openhuman::inference::local::ops::agent_chat).
    ///
    /// The descriptor is threaded onto the turn's run context, so it also
    /// propagates to sub-agents spawned from this session (the same deliberate
    /// isolation the per-profile descriptor has). `None` restores the shared
    /// `action_dir` cwd.
    ///
    /// This only moves the *default* cwd: what the session may read and write is
    /// still decided by its [`SecurityPolicy`](crate::openhuman::security::SecurityPolicy),
    /// so a caller that wants tools rooted somewhere new must build the agent
    /// from a config whose `action_dir` already permits it.
    pub fn set_workspace_descriptor(
        &mut self,
        descriptor: Option<tinyagents_harness::workspace::WorkspaceDescriptor>,
    ) {
        self.workspace_descriptor = descriptor;
    }

    /// Attach an active-run queue for mid-turn steering.
    pub fn set_run_queue(
        &mut self,
        rq: Option<std::sync::Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
    ) {
        self.run_queue = rq;
    }

    /// Restrict which tools the main agent can see and call for this
    /// session. An empty set restores the default "all visible" behavior,
    /// still subject to the configured channel permission policy.
    pub fn set_visible_tool_names(&mut self, names: HashSet<String>) {
        self.visible_tool_names = names;
        self.rebuild_tool_policy_session();
    }

    /// Remove `names` from the main agent's callable set for this session,
    /// leaving every other currently-visible tool untouched.
    ///
    /// The hidden names resolve to `Deny` at the tool-call boundary (via the
    /// rebuilt [`ToolPolicySession`]), not merely absent from the prompt — a
    /// hard execution guarantee even if the model requests the tool anyway.
    ///
    /// When the session currently has *no* visible-tool filter (empty set =
    /// "all visible"), the filter is first seeded from every registered tool
    /// spec so hiding actually **restricts** the set rather than no-opping into
    /// the still-"all visible" empty state. Used by callers that need to drop a
    /// specific dangerous tool from an otherwise-unchanged belt (e.g. the
    /// `flows_build` builder path dropping the live-run `run_flow` tool).
    ///
    /// Caveat: because an empty set is the "all visible" sentinel, hiding *every*
    /// remaining tool collapses back to "all visible". Callers use this to drop
    /// a handful of tools from a much larger belt, where that can't happen.
    pub fn hide_tools(&mut self, names: &[&str]) {
        if self.visible_tool_names.is_empty() {
            self.visible_tool_names = self
                .tool_specs
                .iter()
                .map(|spec| spec.name.clone())
                .collect();
        }
        for name in names {
            self.visible_tool_names.remove(*name);
        }
        // Seeding from `tool_specs` above materialises the "all visible"
        // sentinel into a concrete set, which would re-admit packed tools that
        // the builder withheld. Re-apply the withholding.
        crate::openhuman::tools::toolpacks::strip_packed_from_visible(
            &mut self.visible_tool_names,
            &self.agent_definition_name,
        );
        self.rebuild_tool_policy_session();
    }

    pub(super) fn rebuild_tool_policy_session(&mut self) {
        // Classify the synthesised delegates too: they are advertised to the
        // provider and callable, so a policy snapshot built from the durable
        // registry alone would leave every `delegate_*` tool with no decision
        // at all.
        let all_tools = self.all_tool_refs();
        self.tool_policy_session = ToolPolicyEngine::build_session_from_refs(
            &self.agent_definition_name,
            &self.event_channel,
            "session",
            &self.config.channel_permissions,
            &all_tools,
            &self.visible_tool_names,
        );
        let visible_specs = super::builder::visible_tool_specs_for_policy(
            self.tool_specs.as_slice(),
            &self.visible_tool_names,
            &self.tool_policy_session,
        );
        self.visible_tool_specs = Arc::new(super::builder::dedup_visible_tool_specs(visible_specs));
    }

    /// Clears the agent's conversation history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Set the overrides applied to the **next** [`Self::turn`] call.
    ///
    /// The overrides are consumed at the top of the next turn (they apply to a
    /// single turn, then reset to the default), so a caller running a chat /
    /// small-talk turn calls this immediately before [`Self::turn`]. Callers
    /// that never touch this get the unchanged full-agentic behaviour. See
    /// [`TurnOverrides`](super::types::TurnOverrides) for the fields and the
    /// motivating case (#1725: a bare greeting must not run the task loop nor
    /// inherit a prior task's goal / tools / memory).
    pub fn set_next_turn_overrides(&mut self, overrides: super::types::TurnOverrides) {
        self.pending_turn_overrides = overrides;
    }

    /// Seed the next turn's LLM context from an authoritative message
    /// log (e.g. the web channel's per-thread conversation JSONL).
    ///
    /// Mirrors what [`Self::try_load_session_transcript`] does on a
    /// transcript-file hit, but sources from a caller-supplied list so
    /// resume works even when no transcript file exists for this
    /// agent name (the typical situation right after the
    /// `set_agent_definition_name` / `session_key` rename fix landed —
    /// existing transcripts are written under the old name).
    ///
    /// `messages` is `(role, content)` pairs in chronological order.
    /// Recognised roles: `"user"`, `"agent"` / `"assistant"`. Any
    /// trailing user message that exactly matches `current_user_message`
    /// is dropped — the caller is about to pass that text to
    /// [`Self::run_single`], which will append it to history itself, so
    /// keeping it here would duplicate it on the wire.
    ///
    /// No-ops if the agent already has a history or a cached transcript
    /// (i.e. the per-process session cache is warm). Intended only for
    /// cold-boot priming.
    pub fn seed_resume_from_messages(
        &mut self,
        messages: Vec<(String, String)>,
        current_user_message: &str,
    ) -> Result<()> {
        if !self.history.is_empty() || self.cached_transcript_messages.is_some() {
            return Ok(());
        }
        let mut prior = messages;
        if let Some(last) = prior.last() {
            if last.0 == "user" && last.1.trim() == current_user_message.trim() {
                prior.pop();
            }
        }
        if prior.is_empty() {
            return Ok(());
        }

        // Build the system prompt fresh — there's no persisted prefix
        // to preserve here, and learned-context decoration is skipped
        // intentionally so this fallback path stays synchronous and
        // doesn't fan out to the memory store on every cold-boot turn.
        let learned = crate::openhuman::agent::prompts::LearnedContextData::default();
        let system_prompt = self.build_system_prompt(learned)?;

        let mut cached: Vec<crate::openhuman::agent::messages::ChatMessage> =
            Vec::with_capacity(prior.len() + 1);
        cached.push(crate::openhuman::agent::messages::ChatMessage::system(
            system_prompt,
        ));
        for (role, content) in prior {
            let chat = match role.as_str() {
                "user" => crate::openhuman::agent::messages::ChatMessage::user(content),
                "agent" | "assistant" => {
                    crate::openhuman::agent::messages::ChatMessage::assistant(content)
                }
                // Fall back to user role for unknown senders rather than
                // dropping the message — losing context is worse than
                // mislabelling a system/tool message.
                _ => crate::openhuman::agent::messages::ChatMessage::user(content),
            };
            cached.push(chat);
        }

        let cached_len_before = cached.len();
        let bounded = self.bound_cached_transcript_messages(cached);
        if bounded.len() < cached_len_before {
            log::warn!(
                "[agent] seed_resume_from_messages — bounded cached transcript {} → {} (max_history_messages={})",
                cached_len_before,
                bounded.len(),
                self.config.max_history_messages
            );
        }
        log::info!(
            "[agent] seed_resume_from_messages — primed cached transcript with {} prior messages",
            bounded.len().saturating_sub(1)
        );
        self.cached_transcript_messages = Some(bounded);
        Ok(())
    }

    /// Cold-boot resume for the web-chat path: pre-populate this session's
    /// LLM context from the **full-fidelity** `session_raw/{stem}.jsonl`
    /// transcript for `thread_id`.
    ///
    /// This is the high-fidelity counterpart to
    /// [`Self::seed_resume_from_messages`]. That fallback sources lossy
    /// `(sender, content)` prose from the conversation log, so it drops every
    /// tool call, tool-role result, and reasoning block — after an app restart
    /// the model then "forgets" all its tool interactions. This path instead
    /// routes thread → transcript via
    /// [`transcript::find_root_transcript_for_thread`] and reuses the exact
    /// [`transcript::read_transcript`] +
    /// [`Self::bound_cached_transcript_messages`] machinery as
    /// [`Self::try_load_session_transcript`], so `tool_calls`, `role:"tool"`
    /// messages, and `reasoning_content` all survive the round-trip. The only
    /// difference from `try_load_session_transcript` is the lookup key (thread
    /// id vs. per-thread agent name), so a thread whose transcript was written
    /// under a differently-scoped agent name still resumes.
    ///
    /// Returns `true` when a transcript was found, loaded, and seeded into
    /// `cached_transcript_messages`; `false` (a no-op) when the agent is already
    /// warm, no root transcript exists for the thread, the transcript is empty,
    /// or it fails to parse — the caller then falls back to prose-pair seeding.
    ///
    /// Best-effort like `try_load_session_transcript`: read/parse failures are
    /// logged and reported as `false` rather than propagated. The current turn's
    /// user message is appended later by [`Self::run_single`] / `turn`, so it is
    /// intentionally absent from the loaded prefix — no dedup is needed here (the
    /// on-disk transcript ends at the previous completed turn).
    ///
    /// Goes through the S4 seam like `try_load_session_transcript` (see its doc
    /// comment for why the read is `read_session` and not
    /// `ChatHistory::messages()`), via the locator's `root_for_thread` — the
    /// lookup that resolves by `_meta.thread_id` across *root* transcripts
    /// only. That disambiguation is why it is a locator method rather than
    /// anything a stem-bound handle could offer: several transcripts share one
    /// thread id (every sub-agent spawned within it does).
    pub fn seed_resume_from_thread_transcript(&mut self, thread_id: &str) -> bool {
        if !self.history.is_empty() || self.cached_transcript_messages.is_some() {
            log::debug!(
                "[web-channel] seed_resume_from_thread_transcript no-op — agent already warm \
                 (history_len={}, cached={}) thread={thread_id}",
                self.history.len(),
                self.cached_transcript_messages.is_some()
            );
            return false;
        }

        // The thread's conversation belongs to the THREAD, not the active
        // profile: the locator resolves cross-dir, newest-wins across the
        // shared `session_raw/` and every profile-scoped `session_raw-<id>/`
        // (#5351), so switching profile mid-thread continues the same
        // conversation. See `FileTranscriptLocator::root_for_thread` for why
        // this must not be own-dir-first.
        let Some(handle) = self.session_locator().root_for_thread(thread_id) else {
            log::debug!(
                "[web-channel] no root session_raw transcript for thread={thread_id} in any \
                 (shared or profile-scoped) session_raw dir — falling back to \
                 conversation-log prose seeding"
            );
            return false;
        };
        let path = handle.path().to_path_buf();

        log::info!(
            "[web-channel] cold-boot resume — loading full-fidelity transcript for \
             thread={thread_id} path={}",
            path.display()
        );

        match handle.read_session() {
            // `Ok(None)` (file vanished between discovery and read) folds into
            // the same empty-transcript branch, so the prose-seeding fallback
            // triggers identically.
            Ok(None) => {
                log::debug!(
                    "[web-channel] root transcript for thread={thread_id} is empty — \
                     falling back to prose seeding"
                );
                false
            }
            Ok(Some(session)) => {
                if session.messages.is_empty() {
                    log::debug!(
                        "[web-channel] root transcript for thread={thread_id} is empty — \
                         falling back to prose seeding"
                    );
                    return false;
                }
                let loaded_count = session.messages.len();
                // Count the tool-role results carried into the resumed prefix —
                // the fidelity the prose fallback would have silently dropped.
                let tool_result_msgs = session.messages.iter().filter(|m| m.role == "tool").count();
                let bounded = self.bound_cached_transcript_messages(session.messages);
                if bounded.len() < loaded_count {
                    log::warn!(
                        "[web-channel] resume prefix trimmed from {} to {} messages \
                         (max_history_messages={}) for thread={thread_id}",
                        loaded_count,
                        bounded.len(),
                        self.config.max_history_messages
                    );
                }
                log::info!(
                    "[web-channel] cold-boot resume — primed {} transcript message(s) \
                     ({} tool-role result(s) preserved) for thread={thread_id}",
                    bounded.len(),
                    tool_result_msgs
                );
                self.cached_transcript_messages = Some(bounded);
                true
            }
            Err(err) => {
                log::warn!(
                    "[web-channel] failed to parse root transcript {} for thread={thread_id}: \
                     {err} — falling back to prose seeding",
                    path.display()
                );
                false
            }
        }
    }

    /// Drain and return memory citations collected for the latest completed turn.
    ///
    /// Async because collection runs concurrently with the turn rather than
    /// ahead of it (see `Agent::pending_citations`); this joins whatever is
    /// still in flight. By the time a caller asks, the model round-trip has
    /// already happened, so the recall has normally finished and this does not
    /// wait.
    pub async fn take_last_turn_citations(
        &mut self,
    ) -> Vec<crate::openhuman::memory::agent::memory_loader::MemoryCitation> {
        if let Some(handle) = self.pending_citations.take() {
            match handle.await {
                Ok(citations) => self.last_turn_citations = citations,
                // A panicked or aborted collection must not fail the turn — the
                // citations are decorative, the reply is not.
                Err(err) => {
                    log::warn!("[agent_loop] citation task did not complete: {err}");
                    self.last_turn_citations.clear();
                }
            }
        }
        std::mem::take(&mut self.last_turn_citations)
    }
}
