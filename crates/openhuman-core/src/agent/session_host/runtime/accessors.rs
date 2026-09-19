//! Public getters and setters on [`OpenHumanSessionHost`]: tool registry views, config
//! snapshots, session identity, and the per-session tool-visibility filter
//! (which rebuilds the tool policy snapshot on every change).

use super::super::types::{OpenHumanSessionHost, SessionHostBuilder};
use crate::agent::messages::ConversationMessage;
use crate::memory::Memory;
use crate::tools::agent_policy::ToolPolicyEngine;
use std::collections::HashSet;
use std::sync::Arc;
use tinytools::{Tool, ToolSpec};

impl OpenHumanSessionHost {
    // ─────────────────────────────────────────────────────────────────
    // Small accessors used by `run_single` + `turn` + sub-agent runner
    // ─────────────────────────────────────────────────────────────────

    pub(in crate::agent::session_host) fn event_session_id(&self) -> &str {
        &self.event_session_id
    }

    pub(in crate::agent::session_host) fn event_channel(&self) -> &str {
        &self.event_channel
    }

    /// The agent definition id this session is running
    /// (`"welcome"`, `"orchestrator"`, `"integrations_agent"`, …).
    ///
    /// Exposed so callers that build sessions via
    /// [`OpenHumanSessionHost::from_config_for_agent`] can stamp the resolved id onto
    /// correlation logs and progress events without reaching for the
    /// source `Config`. See [`SessionHostBuilder::agent_definition_name`]
    /// for the full list of downstream surfaces (transcript filename,
    /// transcript metadata header, and `PromptContext::agent_id`) that
    /// read this field.
    pub fn agent_definition_name(&self) -> &str {
        &self.agent_definition_name
    }

    /// Returns a new `SessionHostBuilder`.
    pub fn builder() -> SessionHostBuilder {
        SessionHostBuilder::new()
    }

    /// Clone the agent's model source. Used by the sub-agent runner /
    /// parent-context builder to share the parent's provider instance with
    /// spawned sub-agents (so they share connection pools, retry budgets, and
    /// rate-limit state) — issue #4249, Phase 3 / Motion A.
    pub fn turn_model_source(&self) -> crate::agent::tinyagents::TurnModelSource {
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
    /// Replaced wholesale on every [`OpenHumanSessionHost::refresh_delegation_tools`], so a
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

    /// The full host [`Config`](crate::config::Config) this session
    /// was built with, when it was built through the factory.
    ///
    /// `None` on the bare-builder path (`SessionHostBuilder` without
    /// `AgentFactory`), which is used by tests and by callers assembling a
    /// session by hand. Every capability adapter that needs host config treats
    /// `None` as "not available" rather than loading one itself — see
    /// [`Self::host_capabilities_available`].
    pub fn runtime_config(&self) -> Option<Arc<crate::config::Config>> {
        self.runtime_config.clone()
    }

    /// The definition this session runs under: the one it was built from when
    /// the factory had one, else the process registry's entry for
    /// `agent_definition_id`.
    ///
    /// Prefer this over a bare `AgentDefinitionRegistry::global().get(..)` in
    /// turn-path code that needs the agent's *own* settings (`sandbox_mode`,
    /// `subagents`): a session built from an explicit definition must not have
    /// them replaced by a same-id registry entry.
    pub(crate) fn resolved_definition(
        &self,
    ) -> Option<Arc<crate::agent::harness::definition::AgentDefinition>> {
        self.definition.clone().or_else(|| {
            crate::agent::harness::definition::AgentDefinitionRegistry::global()
                .and_then(|registry| registry.get(&self.agent_definition_id))
                .cloned()
                .map(Arc::new)
        })
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
    pub fn host_agent_memory(&self) -> crate::agent::tinyagents::host::OpenHumanAgentMemory {
        crate::agent::tinyagents::host::OpenHumanAgentMemory::new(self.memory_arc())
    }

    /// OpenHuman's [`ExperienceStore`](tinyagents_harness::host::ExperienceStore)
    /// capability over this session's memory backend.
    pub fn host_experience_store(
        &self,
    ) -> crate::agent::tinyagents::host::OpenHumanExperienceStore {
        crate::agent::tinyagents::host::OpenHumanExperienceStore::new(self.memory_arc())
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
    pub fn workflows(&self) -> &[crate::skills::Workflow] {
        &self.workflows
    }

    /// Active Composio integrations fetched at session start.
    pub fn connected_integrations(&self) -> &[crate::agent::prompts::ConnectedIntegration] {
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
        integrations: Vec<crate::agent::prompts::ConnectedIntegration>,
    ) {
        self.connected_integrations = integrations;
        self.connected_integrations_initialized = true;
        self.last_seen_integrations_hash =
            crate::integrations::composio::connected_set_hash(&self.connected_integrations);
    }

    /// The agent's runtime config snapshot.
    pub fn agent_config(&self) -> &crate::config::AgentConfig {
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

    /// Returns a presentation projection of the runtime-owned history.
    ///
    /// This intentionally returns an owned value: retaining a second borrowed
    /// or mutable `ConversationMessage` accumulator in the host would recreate
    /// the session state now owned by `tinyagents_runtime::Session`.
    pub fn history(&self) -> Vec<ConversationMessage> {
        self.runtime_session
            .as_ref()
            .map(|session| {
                session
                    .history()
                    .iter()
                    .map(crate::agent::message_convert::message_to_native_chat_message)
                    .map(ConversationMessage::Chat)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_event_context(&mut self, session_id: impl Into<String>, channel: impl Into<String>) {
        self.event_session_id = session_id.into();
        self.event_channel = channel.into();
        self.rebuild_tool_policy_session();
        self.sync_runtime_tool_surface();
    }

    /// Bind the OpenHuman conversation thread for the next and subsequent
    /// turns. Empty input intentionally clears the binding.
    pub fn set_thread_id(&mut self, thread_id: Option<impl AsRef<str>>) {
        self.thread_id = thread_id.and_then(|thread_id| {
            let thread_id = thread_id.as_ref().trim();
            (!thread_id.is_empty()).then(|| thread_id.to_owned())
        });
    }

    pub(crate) fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
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
        self.sync_runtime_tool_surface();
    }

    /// Attach a progress event sender for real-time turn updates.
    ///
    /// When set, the turn loop emits [`AgentProgress`] events so
    /// callers (e.g. the web channel) can surface live tool-call and
    /// iteration updates to the UI. Pass `None` to disable.
    pub fn set_on_progress(
        &mut self,
        tx: Option<tokio::sync::mpsc::Sender<crate::agent::progress::AgentProgress>>,
    ) {
        self.on_progress = tx;
    }

    /// Bind this session's acting tools (shell / file / git) to `descriptor`'s
    /// root as their default working directory.
    ///
    /// The post-build counterpart of
    /// [`SessionHostBuilder::workspace_descriptor`](crate::agent::SessionHostBuilder::workspace_descriptor),
    /// for callers that construct the agent through
    /// [`OpenHumanSessionHost::from_config`](crate::agent::OpenHumanSessionHost::from_config) and
    /// therefore never see the builder — notably the per-turn `cwd` of
    /// [`agent_chat`](crate::inference::host_runtime::ops::agent_chat).
    ///
    /// The descriptor is threaded onto the turn's run context, so it also
    /// propagates to sub-agents spawned from this session. `None` restores the
    /// shared `action_dir` cwd.
    ///
    /// This only moves the *default* cwd: what the session may read and write is
    /// still decided by its [`SecurityPolicy`](crate::security::SecurityPolicy),
    /// so a caller that wants tools rooted somewhere new must build the agent
    /// from a config whose `action_dir` already permits it.
    pub fn set_workspace_descriptor(&mut self, descriptor: Option<tinytools::WorkspaceDescriptor>) {
        self.workspace_descriptor = descriptor;
    }

    /// Attach an active-run queue for mid-turn steering.
    pub fn set_run_queue(
        &mut self,
        rq: Option<
            std::sync::Arc<
                tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>,
            >,
        >,
    ) {
        self.run_queue = rq;
    }

    /// Restrict which tools the main agent can see and call for this
    /// session. An empty set restores the default "all visible" behavior,
    /// still subject to the configured channel permission policy.
    pub fn set_visible_tool_names(&mut self, names: HashSet<String>) {
        let auto_include_new_synthesized_tools = names.is_empty();
        self.visible_tool_names = names;
        if self.visible_tool_names.is_empty() {
            self.seed_wildcard_visible_tools();
        }
        self.rebuild_tool_policy_session();
        self.sync_runtime_tool_surface_with_auto_include(Some(auto_include_new_synthesized_tools));
    }

    /// Materialise the wildcard visibility sentinel while preserving the
    /// exposure filters applied during the initial session build.
    fn seed_wildcard_visible_tools(&mut self) {
        self.visible_tool_names = self
            .tool_specs
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        crate::tools::toolpacks::strip_packed_from_visible(
            &mut self.visible_tool_names,
            &self.agent_definition_name,
        );
        let deferred = crate::tools::implementations::meta::strip_deferred_from_visible(
            &mut self.visible_tool_names,
            self.tools.as_slice(),
        );
        crate::tools::implementations::meta::bind_tool_search_index(
            self.tools.as_slice(),
            deferred,
        );
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
            // Durable registry only — synthesised delegates report `Hidden`
            // too and are this belt's hand-off routes.
            self.seed_wildcard_visible_tools();
        }
        for name in names {
            self.visible_tool_names.remove(*name);
        }
        self.rebuild_tool_policy_session();
        self.sync_runtime_tool_surface_with_auto_include(Some(false));
    }

    pub(in crate::agent::session_host) fn rebuild_tool_policy_session(&mut self) {
        // Classify the synthesised delegates too: they are advertised to the
        // provider and callable, so a policy snapshot built from the durable
        // registry alone would leave every `delegate_*` tool with no decision
        // at all.
        let all_tools = self.all_tool_refs();
        let mut session = ToolPolicyEngine::build_session_from_refs(
            &self.agent_definition_name,
            &self.event_channel,
            "session",
            &self.config.channel_permissions,
            &all_tools,
            &self.visible_tool_names,
        );
        // Same narrowing as the builder, re-applied on every rebuild so a
        // delegation refresh cannot reopen a closed pack (#6302).
        crate::tools::toolpacks::close_handed_off_packs(
            &mut session,
            &self.agent_definition_name,
            &all_tools,
        );
        self.tool_policy_session = session;
        let visible_specs = super::super::builder::visible_tool_specs_for_policy(
            self.tool_specs.as_slice(),
            &self.visible_tool_names,
            &self.tool_policy_session,
        );
        self.visible_tool_specs = Arc::new(super::super::builder::dedup_visible_tool_specs(
            visible_specs,
        ));
    }

    /// Clears the agent's conversation history.
    pub fn clear_history(&mut self) {
        // Runtime `Session` owns the only generic history and has no mutable
        // clear operation by design. Dropping this uncommitted/committed
        // composition starts a fresh runtime session on the next public turn.
        self.runtime_session = None;
        self.runtime_state = Arc::new(std::sync::Mutex::new(
            super::super::runtime_session::OpenHumanSessionState::default(),
        ));
    }

    /// Set the overrides applied to the **next** [`Self::turn`] call.
    ///
    /// The overrides are consumed at the top of the next turn (they apply to a
    /// single turn, then reset to the default), so a caller running a chat /
    /// small-talk turn calls this immediately before [`Self::turn`]. Callers
    /// that never touch this get the unchanged full-agentic behaviour. See
    /// [`TurnOverrides`](super::super::types::TurnOverrides) for the fields and the
    /// motivating case (#1725: a bare greeting must not run the task loop nor
    /// inherit a prior task's goal / tools / memory).
    pub fn set_next_turn_overrides(&mut self, overrides: super::super::types::TurnOverrides) {
        self.runtime_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending_turn_overrides = overrides;
    }
}
