//! Tool execution and Composio delegation refresh.

use super::super::agent_tool_exec;
use super::super::types::Agent;
use super::newly_connected_slugs;
use crate::openhuman::agent::dispatcher::ParsedToolCall;
use crate::openhuman::agent::harness;
use crate::openhuman::agent::hooks::ToolCallRecord;
use crate::openhuman::agent::progress::AgentProgress;

use std::sync::Arc;

impl Agent {
    // ─────────────────────────────────────────────────────────────────
    // Per-call tool execution
    // ─────────────────────────────────────────────────────────────────

    /// Executes a single tool call and returns the result and execution record.
    ///
    /// This method:
    /// 1. Emits telemetry events for the start of execution.
    /// 2. Handles the special `spawn_subagent` tool with `fork` context.
    /// 3. Validates tool visibility and availability.
    /// 4. Dispatches to the underlying tool implementation.
    /// 5. Applies per-result byte budgets to prevent context window bloat.
    /// 6. Sanitizes and records the outcome for post-turn hooks.
    pub(in super::super) async fn execute_tool_call(
        &self,
        call: &ParsedToolCall,
        iteration: usize,
    ) -> (
        crate::openhuman::agent::dispatcher::ToolExecutionResult,
        ToolCallRecord,
    ) {
        let normalized_call = super::normalize_tool_call(call);
        let call: &ParsedToolCall = &normalized_call;
        // The per-call execution path lives in the shared
        // [`super::agent_tool_exec::run_agent_tool_call`] so `Agent::turn`
        // (when migrated to the turn engine, via `AgentToolSource`) and any
        // direct caller run the identical logic. Progress is emitted through a
        // `TurnProgress` over this agent's sink. Legacy `run_skill`-wrapped
        // built-in cron tool calls are normalized to direct calls first.
        let progress = super::super::super::engine::TurnProgress::new(self.on_progress.clone());
        let artifact_store =
            crate::openhuman::agent::harness::tool_result_artifacts::ToolResultArtifactStore::new(
                self.action_dir.clone(),
                self.session_key.clone(),
            );
        let ctx = agent_tool_exec::AgentToolExecCtx {
            tools: &self.tools,
            visible_tool_names: &self.visible_tool_names,
            tool_policy_session: &self.tool_policy_session,
            tool_policy: self.tool_policy.as_ref(),
            payload_summarizer: self.payload_summarizer.as_deref(),
            event_session_id: self.event_session_id(),
            event_channel: self.event_channel(),
            agent_definition_id: &self.agent_definition_id,
            prefer_markdown: self.context.prefer_markdown_tool_output(),
            budget_bytes: self.context.tool_result_budget_bytes(),
            compaction_enabled: self.context.compaction_enabled(),
            artifact_store: Some(&artifact_store),
        };
        agent_tool_exec::run_agent_tool_call(&ctx, &progress, call, iteration).await
    }

    /// Executes multiple tool calls in sequence.
    ///
    /// Collects results and execution records for all requested tools in a single batch.
    pub(in super::super) async fn execute_tools(
        &self,
        calls: &[ParsedToolCall],
        iteration: usize,
    ) -> (
        Vec<crate::openhuman::agent::dispatcher::ToolExecutionResult>,
        Vec<ToolCallRecord>,
    ) {
        let mut results = Vec::with_capacity(calls.len());
        let mut records = Vec::with_capacity(calls.len());
        for call in calls {
            let (exec_result, record) = self.execute_tool_call(call, iteration).await;
            results.push(exec_result);
            records.push(record);
        }
        (results, records)
    }

    // ─────────────────────────────────────────────────────────────────
    // Sub-agent context snapshots
    // ─────────────────────────────────────────────────────────────────

    /// Snapshot the parent's runtime so spawned sub-agents can read
    /// it via the [`harness::PARENT_CONTEXT`] task-local.
    pub(in super::super) fn build_parent_execution_context(
        &self,
    ) -> harness::ParentExecutionContext {
        let allowed_subagent_ids = crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
            .and_then(|registry| registry.get(&self.agent_definition_id))
            .map(|definition| {
                definition
                    .subagents
                    .iter()
                    .filter_map(|entry| match entry {
                        crate::openhuman::agent::harness::definition::SubagentEntry::AgentId(id) => {
                            Some(id.clone())
                        }
                        crate::openhuman::agent::harness::definition::SubagentEntry::Skills(wildcard)
                            if wildcard.matches_all() =>
                        {
                            Some("integrations_agent".to_string())
                        }
                        crate::openhuman::agent::harness::definition::SubagentEntry::Skills(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        harness::ParentExecutionContext {
            agent_definition_id: self.agent_definition_id.clone(),
            allowed_subagent_ids,
            provider: Arc::clone(&self.provider),
            all_tools: Arc::clone(&self.tools),
            all_tool_specs: Arc::clone(&self.tool_specs),
            model_name: self.model_name.clone(),
            temperature: self.temperature,
            workspace_dir: self.workspace_dir.clone(),
            memory: Arc::clone(&self.memory),
            agent_config: self.config.clone(),
            workflows: Arc::new(self.workflows.clone()),
            memory_context: Arc::new(self.last_memory_context.clone()),
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
            connected_integrations: self.connected_integrations.clone(),
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
            on_progress: self.on_progress.clone(),
            run_queue: self.run_queue.clone(),
        }
    }

    /// Emit a lifecycle progress event. Uses `send().await` so control
    /// events (turn/iteration boundaries, tool_call_started/completed,
    /// turn_completed) survive downstream backpressure from the
    /// higher-frequency streamed deltas that share the same `on_progress`
    /// channel — dropping one of these would desync the web-channel
    /// progress bridge (e.g. a tool row stuck in `running` forever).
    /// A closed sink is logged and ignored; no progress subscriber is
    /// equivalent to success.
    pub(in super::super) async fn emit_progress(&self, event: AgentProgress) {
        if let Some(ref tx) = self.on_progress {
            if let Err(e) = tx.send(event).await {
                log::warn!("[agent] progress sink closed while emitting lifecycle event: {e}");
            }
        }
    }

    /// Fetches the user's active Composio connections and populates
    /// `self.connected_integrations` so the system prompt can surface them.
    ///
    /// Delegates to the shared [`crate::openhuman::composio::fetch_connected_integrations`]
    /// which is the single source of truth for integration discovery.
    ///
    /// **No session-scoped Composio client is cached on the agent any
    /// more (#1710 Wave 2)**. Every downstream caller that needs to
    /// dispatch a Composio action now resolves a fresh client via
    /// [`crate::openhuman::composio::client::create_composio_client`]
    /// at call time so the live `composio.mode` toggle is honoured
    /// without rebuilding the session — see `ComposioActionTool`,
    /// `ProviderContext::execute`, the 5 migrated agent tools in
    /// `composio/tools.rs`, and the spawn-time per-action tool build
    /// path in `subagent_runner/ops.rs`.
    pub async fn fetch_connected_integrations(&mut self) {
        let config = match self.integration_runtime_config.clone() {
            Some(config) => config,
            None => match crate::openhuman::config::Config::load_or_init().await {
                Ok(config) => config,
                Err(e) => {
                    log::debug!(
                        "[agent] skipping connected integrations fetch: config load failed: {e}"
                    );
                    return;
                }
            },
        };
        self.connected_integrations =
            crate::openhuman::composio::fetch_connected_integrations(&config).await;
        self.connected_integrations_initialized = true;
    }

    /// Lazily attach this session to the global event bus so it can
    /// observe `ComposioIntegrationsChanged` notifications.
    pub(in super::super) fn ensure_composio_integrations_listener(&mut self) {
        if self.composio_integrations_rx.is_some() {
            return;
        }
        if let Some(bus) = crate::core::event_bus::global() {
            self.composio_integrations_rx = Some(bus.raw_receiver());
            log::debug!(
                "[agent_loop] armed composio integrations listener for session='{}'",
                self.event_session_id
            );
        }
    }

    /// Drain pending `ComposioIntegrationsChanged` events.
    ///
    /// Returns `true` when we observed at least one relevant event (or lag) and
    /// should re-check cached integrations before the next provider call.
    pub(in super::super) fn drain_composio_integrations_changed_events(&mut self) -> bool {
        self.ensure_composio_integrations_listener();
        let Some(rx) = self.composio_integrations_rx.as_mut() else {
            return false;
        };
        use tokio::sync::broadcast::error::TryRecvError;

        let mut saw_signal = false;
        let mut closed = false;
        loop {
            match rx.try_recv() {
                Ok(crate::core::event_bus::DomainEvent::ComposioIntegrationsChanged {
                    toolkits,
                }) => {
                    saw_signal = true;
                    log::info!(
                        "[agent_loop] received composio integrations changed event (active_toolkits={:?})",
                        toolkits
                    );
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(skipped)) => {
                    saw_signal = true;
                    log::warn!(
                        "[agent_loop] composio integrations listener lagged by {} event(s); forcing cache re-check",
                        skipped
                    );
                }
                Err(TryRecvError::Closed) => {
                    closed = true;
                    break;
                }
            }
        }
        if closed {
            self.composio_integrations_rx = None;
        }
        saw_signal
    }

    /// Lazily attach this session to the global event bus so it can observe
    /// [`crate::core::event_bus::DomainEvent::WorkflowsChanged`] (skill
    /// install / uninstall / create). Mirror of
    /// [`Self::ensure_composio_integrations_listener`].
    pub(in super::super) fn ensure_skill_events_listener(&mut self) {
        if self.skill_events_rx.is_some() {
            return;
        }
        if let Some(bus) = crate::core::event_bus::global() {
            self.skill_events_rx = Some(bus.raw_receiver());
            log::debug!(
                "[agent_loop] armed installed-skills listener for session='{}'",
                self.event_session_id
            );
        }
    }

    /// Drain pending [`crate::core::event_bus::DomainEvent::WorkflowsChanged`]
    /// events. Returns `true` when at least one was observed (or the listener
    /// lagged) and the caller should re-scan the installed skill set via
    /// [`Self::refresh_workflows`]. Mirror of
    /// [`Self::drain_composio_integrations_changed_events`].
    pub(in super::super) fn drain_skill_events(&mut self) -> bool {
        self.ensure_skill_events_listener();
        let Some(rx) = self.skill_events_rx.as_mut() else {
            return false;
        };
        use tokio::sync::broadcast::error::TryRecvError;

        let mut saw_signal = false;
        let mut closed = false;
        loop {
            match rx.try_recv() {
                Ok(crate::core::event_bus::DomainEvent::WorkflowsChanged { reason }) => {
                    saw_signal = true;
                    log::info!("[agent_loop] received installed-skills changed event ({reason})");
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(skipped)) => {
                    saw_signal = true;
                    log::warn!(
                        "[agent_loop] installed-skills listener lagged by {} event(s); forcing catalogue re-check",
                        skipped
                    );
                }
                Err(TryRecvError::Closed) => {
                    closed = true;
                    break;
                }
            }
        }
        if closed {
            self.skill_events_rx = None;
        }
        saw_signal
    }

    /// Reconcile the session's delegation schema against the latest cached
    /// integrations snapshot. Returns `true` only when a refresh applied.
    pub(in super::super) fn refresh_delegation_tools_from_cached_integrations(
        &mut self,
        trigger: &str,
    ) -> bool {
        let Some(cfg) = self.integration_runtime_config.as_ref() else {
            return false;
        };
        let Some(cache_view) = crate::openhuman::composio::cached_active_integrations(cfg) else {
            return false;
        };

        let new_hash = crate::openhuman::composio::connected_set_hash(&cache_view);
        if new_hash == self.last_seen_integrations_hash {
            return false;
        }

        log::info!(
            "[agent_loop] composio set changed ({trigger}) hash {:x} -> {:x}; refreshing delegation schema (system prompt unchanged for KV cache)",
            self.last_seen_integrations_hash,
            new_hash
        );

        let prev_integrations = std::mem::replace(&mut self.connected_integrations, cache_view);
        if self.refresh_delegation_tools() {
            self.last_seen_integrations_hash = new_hash;
            self.connected_integrations_initialized = true;
            // Surface newly-connected toolkits onto the next user message so
            // the model acts on them on the FIRST post-connect ask instead of
            // refusing from stale chat context. Schema-only refresh already
            // updated the enum; this closes the prose/decision gap.
            let connected_slugs: Vec<String> = self
                .connected_integrations
                .iter()
                .map(|i| i.toolkit.clone())
                .collect();
            // Append (don't overwrite) so a second connect before the next
            // user turn doesn't drop the first one's announcement. Slugs are
            // already de-duped against `announced_integrations`, but guard the
            // pending list too in case the same slug is re-queued.
            for slug in newly_connected_slugs(&connected_slugs, &mut self.announced_integrations) {
                if !self.pending_integration_announcement.contains(&slug) {
                    self.pending_integration_announcement.push(slug);
                }
            }
            true
        } else {
            self.connected_integrations = prev_integrations;
            false
        }
    }

    /// Reconcile the tracked installed-skill set ([`Self::workflows`]) against
    /// what is on disk, so a skill installed/uninstalled mid-session can be
    /// surfaced to the model without a session restart.
    ///
    /// Note the system-prompt `## Installed Skills` block is frozen at turn 1
    /// (KV-cache stability — it is only built when history is empty), so this
    /// does NOT rebuild that block for the live session. Instead — exactly like
    /// [`Self::refresh_delegation_tools_from_cached_integrations`] / the MCP
    /// mid-session mechanism — genuinely-new skill ids (present on disk but not
    /// in the prior snapshot) are parked in [`Self::pending_skill_announcement`]
    /// (announced once via [`Self::announced_skills`]) and surfaced on the next
    /// user turn; `run_skill` then loads/runs them fresh from disk. Updating the
    /// tracked slice keeps the next diff correct and feeds a *fresh* session's
    /// rendered catalogue.
    ///
    /// Returns `true` when the installed set changed. Cheap no-op when it
    /// hasn't: a directory scan plus an id-set comparison, no prompt rebuild.
    pub(in super::super) fn refresh_workflows(&mut self, trigger: &str) -> bool {
        let id_of = |w: &crate::openhuman::workflows::Workflow| -> String {
            if w.dir_name.is_empty() {
                w.name.clone()
            } else {
                w.dir_name.clone()
            }
        };
        let latest = crate::openhuman::workflows::load_workflow_metadata(&self.workspace_dir);
        let current_ids: std::collections::HashSet<String> =
            self.workflows.iter().map(&id_of).collect();
        let latest_ids: std::collections::HashSet<String> = latest.iter().map(&id_of).collect();
        if current_ids == latest_ids {
            return false;
        }
        // Newly-present skills (on disk now, absent from the prior snapshot),
        // announced at most once this session.
        let newly: Vec<String> = latest_ids
            .difference(&current_ids)
            .filter(|id| self.announced_skills.insert((*id).clone()))
            .cloned()
            .collect();
        // Skills removed from disk since the last snapshot: retract them so the
        // model stops routing `run_skill` calls to skills that no longer exist.
        // The frozen `## Installed Skills` system-prompt block cannot be updated
        // mid-session (KV-cache stability), so the retraction note on the user
        // turn is the only signal the model gets — mirrors the install path.
        // Clear from `announced_skills` so a re-install later is announced fresh.
        let removed: Vec<String> = current_ids.difference(&latest_ids).cloned().collect();
        for id in &removed {
            self.announced_skills.remove(id);
        }
        log::info!(
            "[agent_loop] installed-skills set changed ({trigger}): {} -> {} skills (new={} removed={}); updating tracked set + parking notes (system-prompt catalogue frozen for KV cache)",
            self.workflows.len(),
            latest.len(),
            newly.len(),
            removed.len(),
        );
        self.workflows = latest;
        for id in newly {
            // A re-install after a still-pending retraction cancels the
            // retraction: the skill is present again, so drop the stale "gone"
            // note and announce it instead.
            self.pending_skill_retraction.retain(|p| p != &id);
            if !self.pending_skill_announcement.contains(&id) {
                self.pending_skill_announcement.push(id);
            }
        }
        for id in removed {
            // If the skill was installed and uninstalled before its
            // announcement ever surfaced, the model never saw it as available —
            // drop the pending announcement so we don't emit a contradictory
            // "installed" + "retracted" pair on the same user turn.
            self.pending_skill_announcement.retain(|p| p != &id);
            if !self.pending_skill_retraction.contains(&id) {
                self.pending_skill_retraction.push(id);
            }
        }
        true
    }

    /// Test-only: installed-skill ids currently in the catalogue snapshot
    /// (`dir_name`, falling back to `name`). Lets `refresh_workflows` tests
    /// assert through a method instead of touching private fields.
    #[cfg(test)]
    pub(in super::super) fn test_workflow_ids(&self) -> Vec<String> {
        self.workflows
            .iter()
            .map(|w| {
                if w.dir_name.is_empty() {
                    w.name.clone()
                } else {
                    w.dir_name.clone()
                }
            })
            .collect()
    }

    /// Test-only: skill ids parked for the next-turn `[skills update]`
    /// announcement by `refresh_workflows`.
    #[cfg(test)]
    pub(in super::super) fn test_pending_skill_announcement(&self) -> &[String] {
        &self.pending_skill_announcement
    }

    /// Test-only: skill ids parked for the next-turn `[skills retracted]`
    /// retraction note by `refresh_workflows`.
    #[cfg(test)]
    pub(in super::super) fn test_pending_skill_retraction(&self) -> &[String] {
        &self.pending_skill_retraction
    }

    /// Test-only: inject a specific skill-events receiver (e.g. one whose
    /// sender has been dropped) so `drain_skill_events`' `Closed` arm is
    /// reachable without the global bus singleton.
    #[cfg(test)]
    pub(in super::super) fn set_skill_events_rx_for_test(
        &mut self,
        rx: tokio::sync::broadcast::Receiver<crate::core::event_bus::DomainEvent>,
    ) {
        self.skill_events_rx = Some(rx);
    }

    /// Test-only: whether the skill-events listener is currently armed.
    #[cfg(test)]
    pub(in super::super) fn has_skill_events_rx(&self) -> bool {
        self.skill_events_rx.is_some()
    }

    /// Test-only: inject a specific composio-integrations receiver so the
    /// drain path can be exercised against an isolated bus instead of the
    /// global singleton (which other parallel tests publish into, racing the
    /// "drained after one pass" assertion). Mirror of
    /// [`Self::set_skill_events_rx_for_test`].
    #[cfg(test)]
    pub(in super::super) fn set_composio_integrations_rx_for_test(
        &mut self,
        rx: tokio::sync::broadcast::Receiver<crate::core::event_bus::DomainEvent>,
    ) {
        self.composio_integrations_rx = Some(rx);
    }

    /// Re-synthesise `delegate_*` tools for the orchestrator's `subagents`
    /// declaration using the live `connected_integrations` slice, and
    /// reconcile the resulting set into `self.tools` / `self.tool_specs` /
    /// `self.visible_tool_specs` / `self.visible_tool_names`.
    ///
    /// **Reconciliation strategy** — full rebuild of the synthesised
    /// subset:
    ///
    ///   1. Drop every tool whose name was in [`Self::synthesized_tool_names`]
    ///      from the previous synthesis. Direct tools (`query_memory`,
    ///      `cron_add`, …) are untouched because their names are not in
    ///      that set.
    ///   2. Append the freshly collected synthesis output verbatim.
    ///   3. Replace `synthesized_tool_names` with the new set so the
    ///      next refresh has a clean mask to undo.
    ///
    /// This is safer than appending-only or strict-diff reconcile:
    ///
    ///   * Stale tools after a revoke can never leak — anything from the
    ///     previous synthesis is unconditionally dropped, the new set is
    ///     authoritative.
    ///   * Direct tools can never be accidentally removed — only names
    ///     in `synthesized_tool_names` are touched.
    ///   * Duplicate registration is impossible — retain+extend
    ///     guarantees every final entry is either a non-synthesised
    ///     direct tool or a member of the fresh `synthed` set.
    ///
    /// **When to call**: on turn 1 only when the session was built
    /// without a prewarmed Composio cache snapshot, and on any
    /// subsequent turn where the connection set has changed since the
    /// last reconcile (detected via
    /// [`Self::last_seen_integrations_hash`] vs.
    /// [`crate::openhuman::composio::cached_active_integrations`]).
    ///
    /// **Shared-Arc behavior**: when `self.tools` is currently shared
    /// (e.g. an in-flight turn cloned the Arc into its tool source), we
    /// still refresh `self.tool_specs` / `self.visible_tool_specs` so the
    /// provider-facing schema updates immediately. The executable tool
    /// registry is refreshed only when `self.tools` has unique ownership.
    /// This keeps same-turn routing unblocked while preserving ownership
    /// safety for non-cloneable `Box<dyn Tool>` values.
    ///
    /// **Return value** — `true` when schema reconciliation succeeded (or
    /// no reconcile was needed). Returns `false` only when a non-shared
    /// reconcile path failed unexpectedly.
    pub fn refresh_delegation_tools(&mut self) -> bool {
        use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
        use crate::openhuman::tools::orchestrator_tools::collect_orchestrator_tools;

        let Some(reg) = AgentDefinitionRegistry::global() else {
            // No registry — there's nothing we can do until the
            // registry is initialised. The agent's surface stays at
            // whatever the builder produced; callers can safely treat
            // this as "no reconcile needed right now".
            return true;
        };
        let Some(def) = reg.get(&self.agent_definition_id) else {
            log::debug!(
                "[agent] refresh_delegation_tools: definition '{}' not in registry — skipping",
                self.agent_definition_id
            );
            return true;
        };
        if def.subagents.is_empty() {
            return true;
        }

        let synthed = collect_orchestrator_tools(def, reg, &self.connected_integrations);
        let synthed_names: std::collections::HashSet<String> =
            synthed.iter().map(|t| t.name().to_string()).collect();
        let synthed_specs: Vec<crate::openhuman::tools::ToolSpec> =
            synthed.iter().map(|t| t.spec()).collect();

        // Skip mutation when neither the previous nor the next synthesis
        // produced any names — saves work on agents without dynamic
        // delegation.
        if self.synthesized_tool_names.is_empty() && synthed_names.is_empty() {
            return true;
        }

        // Mask of the previous synthesis — the names whose `tool_specs` are
        // currently live (this set is kept in lock-step with `tool_specs`).
        let old_synth = std::mem::take(&mut self.synthesized_tool_names);

        // `tool_specs` are plain data and therefore cloneable; we can always
        // reconcile schema even when the Arc is shared. Drop exactly the
        // previous synthesised spec set, then append the fresh one.
        {
            let specs_vec = Arc::make_mut(&mut self.tool_specs);
            specs_vec.retain(|s| !old_synth.contains(&s.name));
            specs_vec.extend(synthed_specs);
        }

        // `tools` contains non-cloneable trait objects. Reconcile it only when
        // uniquely owned. The set of stale synthesised *instances* to drop is
        // the previous synthesis (`old_synth`) plus any instances a prior
        // shared-Arc refresh couldn't remove (`pending_synthesized_tools_mask`).
        let tools_remove_mask: std::collections::HashSet<String> = old_synth
            .iter()
            .chain(self.pending_synthesized_tools_mask.iter())
            .cloned()
            .collect();
        let tools_reconciled = if let Some(tools_vec) = Arc::get_mut(&mut self.tools) {
            tools_vec.retain(|t| !tools_remove_mask.contains(t.name()));
            tools_vec.extend(synthed);
            // `tools` now matches `tool_specs` exactly — nothing pending.
            self.pending_synthesized_tools_mask.clear();
            true
        } else {
            // Schema (`tool_specs`) was updated to the new set, but the stale
            // tool *instances* still sit in `self.tools`. Record their names
            // so the next unique-owner refresh removes them. Crucially we do
            // NOT roll `synthesized_tool_names` back to `old_synth` here — that
            // would desync it from `tool_specs` and cause duplicate specs on
            // the following refresh (#3044).
            self.pending_synthesized_tools_mask = tools_remove_mask;
            log::warn!(
                "[agent] refresh_delegation_tools: tools Arc is shared — refreshed schema only \
                 ({} synthesised tool name(s)); {} stale tool instance(s) pending removal on the next unique-owner refresh",
                synthed_names.len(),
                self.pending_synthesized_tools_mask.len()
            );
            false
        };

        // `visible_tool_names` carries an explicit allowlist for
        // [`ToolScope::Named`] agents. Drop the previously-synthesised
        // names and add the new ones so the visible set tracks the
        // tool list. Wildcard-scope agents keep this empty ("no
        // filter") and never need touching.
        if !self.visible_tool_names.is_empty() {
            for name in &old_synth {
                self.visible_tool_names.remove(name);
            }
            for name in &synthed_names {
                self.visible_tool_names.insert(name.clone());
            }
        }

        // Rebuild the visible-spec cache from the new tool_specs so the
        // next provider call carries the reconciled schema. Dedup
        // afterward so a delegate synthesised here (e.g.
        // `delegate_name = "research"`) doesn't collide with a
        // same-named skill tool on the wire — Anthropic 400s on dup
        // tool names where OpenHuman's backend silently accepts.
        self.rebuild_tool_policy_session();

        // Compute add/remove deltas for the log line — useful when
        // diagnosing a Composio connect/revoke that should have rebuilt
        // the surface but didn't. Materialise to owned `Vec<String>`
        // so we can move `synthed_names` into `self.synthesized_tool_names`
        // below without the log-statement reborrow blocking the move.
        let added: Vec<String> = synthed_names
            .iter()
            .filter(|n| !old_synth.contains(n.as_str()))
            .cloned()
            .collect();
        let removed: Vec<String> = old_synth
            .iter()
            .filter(|n| !synthed_names.contains(n.as_str()))
            .cloned()
            .collect();

        // `tool_specs` always reconciled to the new set, so the name mask must
        // track that set unconditionally — whether or not `tools` (the
        // executable instances) could be reconciled this pass.
        self.synthesized_tool_names = synthed_names.clone();

        log::info!(
            "[agent] refresh_delegation_tools: reconciled delegation schema for agent '{}' (display='{}'); now {} synthesised tool name(s); added={:?} removed={:?} tools_reconciled={} pending_tool_instances={}",
            self.agent_definition_id,
            self.agent_definition_name,
            synthed_names.len(),
            added,
            removed,
            tools_reconciled,
            self.pending_synthesized_tools_mask.len()
        );
        true
    }
}
