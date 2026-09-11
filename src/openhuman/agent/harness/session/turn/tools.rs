//! Tool execution and Composio delegation refresh.

use super::super::types::Agent;
use super::newly_connected_slugs;
use crate::openhuman::agent::harness;
use crate::openhuman::agent::progress::AgentProgress;

use std::sync::Arc;

/// One turn's tool inputs: the durable registry, the synthesised delegation
/// set, and the callable-name allowlist. See [`Agent::turn_tool_sets`].
type TurnToolSets = (
    Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>,
    Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>,
    std::collections::HashSet<String>,
);

impl Agent {
    // ─────────────────────────────────────────────────────────────────
    // Sub-agent context snapshots
    // ─────────────────────────────────────────────────────────────────

    /// Snapshot the parent's runtime so spawned sub-agents can read
    /// it via the [`harness::PARENT_CONTEXT`] task-local.
    pub(super) fn build_parent_execution_context(&self) -> harness::ParentExecutionContext {
        // Prefer an ambient `current_parent()` descriptor (a nested subagent
        // inherits its enclosing worktree/profile workspace threaded down the
        // spawn chain). Fall back to THIS session agent's own descriptor: on a
        // ROOT chat turn `current_parent()` is `None`, so without the fallback a
        // dedicated-workspace profile's descriptor (`<action_dir>/profiles/<id>`,
        // set on the Agent at build time) would never reach delegated subagents
        // spawned via `spawn_subagent` / `spawn_async_subagent`, and they'd drop
        // to the shared `action_dir` — the profile isolation would silently not
        // apply to common delegated writes.
        let workspace_descriptor = harness::current_parent()
            .and_then(|parent| parent.workspace_descriptor)
            .or_else(|| self.workspace_descriptor.clone());
        if let Some(descriptor) = workspace_descriptor.as_ref() {
            tracing::debug!(
                root = %descriptor.root.display(),
                policy_id = %descriptor.policy_id,
                "[agent_loop] snapshotting workspace descriptor for parent context (ambient or own)"
            );
        }
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
            turn_model_source: self.turn_model_source.clone(),
            all_tools: Arc::clone(&self.tools),
            // The durable registry's own specs, index for index with
            // `all_tools` — never the synthesised delegation specs, which a
            // child holds no instance for and must not see (#4452).
            all_tool_specs: Arc::clone(&self.durable_tool_specs),
            visible_tool_names: self
                .visible_tool_specs
                .iter()
                .map(|spec| spec.name.clone())
                .collect(),
            visible_tool_specs: Arc::clone(&self.visible_tool_specs),
            subagent_tool_ceiling_names: self.subagent_tool_ceiling_names.clone(),
            model_name: self.model_name.clone(),
            temperature: self.temperature,
            workspace_dir: self.workspace_dir.clone(),
            workspace_descriptor,
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

    /// The tool sets and callable-name allowlist for one turn.
    ///
    /// Returns `(durable tools, synthesised delegation tools, visible names)`.
    /// The two tool sets stay separate all the way to dispatch — see
    /// [`Agent::synthesized_tools`] for why they are not one `Arc`.
    ///
    /// `suppress_tools` is the per-turn scope override (#1725): a chat /
    /// small-talk turn runs with an EMPTY tool set, so the provider request
    /// carries no tool schema and the model answers in a single call. The
    /// agent's durable fields are left untouched either way — the next
    /// un-overridden turn gets the full toolbelt back.
    pub(super) fn turn_tool_sets(&self, suppress_tools: bool) -> TurnToolSets {
        if suppress_tools {
            return (
                Arc::new(Vec::new()),
                Arc::new(Vec::new()),
                std::collections::HashSet::new(),
            );
        }
        (
            Arc::clone(&self.tools),
            Arc::clone(&self.synthesized_tools),
            self.visible_tool_names.clone(),
        )
    }

    /// Emit a lifecycle progress event. Uses `send().await` so control
    /// events (turn/iteration boundaries, tool_call_started/completed,
    /// turn_completed) survive downstream backpressure from the
    /// higher-frequency streamed deltas that share the same `on_progress`
    /// channel — dropping one of these would desync the web-channel
    /// progress bridge (e.g. a tool row stuck in `running` forever).
    /// A closed sink is logged and ignored; no progress subscriber is
    /// equivalent to success.
    pub(super) async fn emit_progress(&self, event: AgentProgress) {
        if let Some(ref tx) = self.on_progress {
            if let Err(e) = tx.send(event).await {
                log::warn!("[agent] progress sink closed while emitting lifecycle event: {e}");
            }
        }
    }

    /// Fetches the user's active Composio connections and populates
    /// `self.connected_integrations` so the system prompt can surface them.
    ///
    /// Delegates to the shared [`crate::openhuman::integrations::composio::fetch_connected_integrations`]
    /// which is the single source of truth for integration discovery.
    ///
    /// **No session-scoped Composio client is cached on the agent any
    /// more (#1710 Wave 2)**. Every downstream caller that needs to
    /// dispatch a Composio action now resolves a fresh client via
    /// [`crate::openhuman::integrations::composio::client::create_composio_client`]
    /// at call time so the live `composio.mode` toggle is honoured
    /// without rebuilding the session — see `ComposioActionTool`,
    /// `ProviderContext::execute`, the 5 migrated agent tools in
    /// `composio/tools.rs`, and the spawn-time per-action tool build
    /// path in `subagent_runner/ops.rs`.
    pub async fn fetch_connected_integrations(&mut self) {
        let config = match self.runtime_config.clone() {
            Some(config) => config,
            None => match crate::openhuman::config::Config::load_or_init().await {
                Ok(config) => Arc::new(config),
                Err(e) => {
                    log::debug!(
                        "[agent] skipping connected integrations fetch: config load failed: {e}"
                    );
                    return;
                }
            },
        };
        self.connected_integrations =
            crate::openhuman::integrations::composio::fetch_connected_integrations(&config).await;
        self.connected_integrations_initialized = true;
    }

    /// Lazily attach this session to the global event bus so it can
    /// observe `ComposioIntegrationsChanged` notifications.
    pub(super) fn ensure_composio_integrations_listener(&mut self) {
        if self.composio_integrations_rx.is_some() {
            return;
        }
        if let Some(bus) = crate::core::bus::BUS.get() {
            self.composio_integrations_rx = Some(bus.receiver());
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
        use tinybus::TryRecvError;

        let mut saw_signal = false;
        let mut closed = false;
        loop {
            match rx.try_recv() {
                Ok(crate::core::events::DomainEvent::ComposioIntegrationsChanged { toolkits }) => {
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
    /// [`crate::core::events::DomainEvent::WorkflowsChanged`] (skill
    /// install / uninstall / create). Mirror of
    /// [`Self::ensure_composio_integrations_listener`].
    pub(super) fn ensure_skill_events_listener(&mut self) {
        if self.skill_events_rx.is_some() {
            return;
        }
        if let Some(bus) = crate::core::bus::BUS.get() {
            self.skill_events_rx = Some(bus.receiver());
            log::debug!(
                "[agent_loop] armed installed-skills listener for session='{}'",
                self.event_session_id
            );
        }
    }

    /// Drain pending [`crate::core::events::DomainEvent::WorkflowsChanged`]
    /// events. Returns `true` when at least one was observed (or the listener
    /// lagged) and the caller should re-scan the installed skill set via
    /// [`Self::refresh_workflows`]. Mirror of
    /// [`Self::drain_composio_integrations_changed_events`].
    pub(in super::super) fn drain_skill_events(&mut self) -> bool {
        self.ensure_skill_events_listener();
        let Some(rx) = self.skill_events_rx.as_mut() else {
            return false;
        };
        use tinybus::TryRecvError;

        let mut saw_signal = false;
        let mut closed = false;
        loop {
            match rx.try_recv() {
                Ok(crate::core::events::DomainEvent::WorkflowsChanged { reason }) => {
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
    pub(super) fn refresh_delegation_tools_from_cached_integrations(
        &mut self,
        trigger: &str,
    ) -> bool {
        let Some(cfg) = self.runtime_config.as_ref() else {
            return false;
        };
        let Some(cache_view) =
            crate::openhuman::integrations::composio::cached_active_integrations(cfg)
        else {
            return false;
        };

        let new_hash = crate::openhuman::integrations::composio::connected_set_hash(&cache_view);
        if new_hash == self.last_seen_integrations_hash {
            return false;
        }

        log::info!(
            "[agent_loop] composio set changed ({trigger}) hash {:x} -> {:x}; refreshing delegation schema (system prompt unchanged for KV cache)",
            self.last_seen_integrations_hash,
            new_hash
        );

        // No rollback path: `refresh_delegation_tools` reconciles the specs and
        // the executable instances in one pass and cannot half-apply, so there
        // is no failed state to restore `connected_integrations` from.
        self.connected_integrations = cache_view;
        self.refresh_delegation_tools();
        self.last_seen_integrations_hash = new_hash;
        self.connected_integrations_initialized = true;
        // Surface newly-connected toolkits onto the next user message so
        // the model acts on them on the FIRST post-connect ask instead of
        // refusing from stale chat context. The refresh above already
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
        let id_of = |w: &crate::openhuman::skills::Workflow| -> String {
            if w.dir_name.is_empty() {
                w.name.clone()
            } else {
                w.dir_name.clone()
            }
        };
        // Keep the mid-session refresh consistent with the initial catalog
        // (built in the session factory): include the active profile's private
        // skills root so profile-local installs are tracked/announced too. `None`
        // for the profile-less session reproduces the prior behaviour.
        let profile_skills_root = self.active_profile_id.as_deref().and_then(|id| {
            crate::openhuman::agent::profiles::profile_skills_root(&self.workspace_dir, id)
        });
        // An invalid/absent active profile id silently falls back to shared
        // discovery. Log the branch id-free (boolean only, never the profile id or
        // resolved path) per the observability convention for new/changed flows.
        let profile_local_skills_active = profile_skills_root.is_some();
        log::debug!(
            "[agent_loop] refreshing installed-skills metadata (trigger={trigger}, profile_local_skills_active={profile_local_skills_active})"
        );
        let latest = crate::openhuman::skills::load_workflow_metadata_for_profile(
            &self.workspace_dir,
            profile_skills_root.as_deref(),
        );
        log::debug!(
            "[agent_loop] refreshed installed-skills metadata (trigger={trigger}, profile_local_skills_active={profile_local_skills_active}, workflow_count={})",
            latest.len()
        );
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
        rx: tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
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
        rx: tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
    ) {
        self.composio_integrations_rx = Some(rx);
    }

    /// Re-synthesise `delegate_*` tools for the orchestrator's `subagents`
    /// declaration using the live `connected_integrations` slice, and
    /// reconcile the resulting set into `self.synthesized_tools` /
    /// `self.tool_specs` / `self.visible_tool_specs` / `self.visible_tool_names`.
    /// `self.tools` is never touched.
    ///
    /// **Reconciliation strategy** — full rebuild of the synthesised
    /// subset:
    ///
    ///   1. Drop every spec whose name was in [`Self::synthesized_tool_names`]
    ///      from the previous synthesis. Direct tools (`query_memory`,
    ///      `cron_add`, …) are untouched because their names are not in
    ///      that set.
    ///   2. Append the fresh specs, and replace [`Self::synthesized_tools`]
    ///      with the fresh instances — minus any name a durable tool owns,
    ///      which the durable tool keeps (the same rule the builder applies).
    ///   3. Replace `synthesized_tool_names` with the new set so the
    ///      next refresh has a clean mask to undo.
    ///
    /// This is safer than appending-only or strict-diff reconcile:
    ///
    ///   * Stale tools after a revoke can never leak — anything from the
    ///     previous synthesis is unconditionally dropped, the new set is
    ///     authoritative.
    ///   * Direct tools can never be accidentally removed — only names
    ///     in `synthesized_tool_names` are touched, and a durable name is
    ///     never added to that mask.
    ///   * Duplicate registration is impossible — the fresh set replaces the
    ///     previous one wholesale and is disjoint from `self.tools`, so a
    ///     name is registered at most once across both sets.
    ///
    /// **When to call**: on turn 1 only when the session was built
    /// without a prewarmed Composio cache snapshot, and on any
    /// subsequent turn where the connection set has changed since the
    /// last reconcile (detected via
    /// [`Self::last_seen_integrations_hash`] vs.
    /// [`crate::openhuman::integrations::composio::cached_active_integrations`]).
    ///
    /// **Concurrency**: this cannot fail on a shared session. The synthesised
    /// instances live in their own [`Agent::synthesized_tools`] `Arc`, which is
    /// *replaced* rather than mutated in place — so an in-flight turn or a
    /// spawned sub-agent holding a clone never blocks reconciliation. Those
    /// readers keep the previous, self-consistent set for the rest of their
    /// turn; the superseded instances are freed when the last of them drops.
    ///
    /// This is what makes the schema and the executable surface inseparable.
    /// Reconciling into `self.tools` instead required `Arc::get_mut`, which
    /// fails under exactly that sharing — and the old code proceeded to
    /// reconcile `tool_specs` anyway, so the two halves drifted: a newly
    /// connected toolkit's delegate had a spec with no instance (and no policy
    /// decision, so the fail-closed visibility filter hid it — silently missing
    /// until a unique-owner refresh) while a revoked toolkit's delegate kept its
    /// instance with no spec — still registered and callable (#6145).
    ///
    /// Returns nothing: with the synthesised set held in its own `Arc` there is
    /// no longer a way for this to half-apply, so the `bool` it used to hand
    /// back — and the caller rollback keyed on it — had no reachable `false`.
    pub fn refresh_delegation_tools(&mut self) {
        use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
        use crate::openhuman::tools::orchestrator_tools::collect_orchestrator_tools;

        let Some(reg) = AgentDefinitionRegistry::global() else {
            // No registry — there's nothing we can do until the
            // registry is initialised. The agent's surface stays at
            // whatever the builder produced.
            return;
        };
        let Some(def) = reg.get(&self.agent_definition_id) else {
            log::debug!(
                "[agent] refresh_delegation_tools: definition '{}' not in registry — skipping",
                self.agent_definition_id
            );
            return;
        };
        if def.subagents.is_empty() {
            return;
        }

        // A durable name wins a collision, exactly as at build time. Filtering
        // here also keeps such a name out of the mask below, so the spec
        // `retain` can never withdraw a durable tool's spec.
        let synthed = super::super::builder::drop_synthesized_name_collisions(
            &self.tools,
            collect_orchestrator_tools(def, reg, &self.connected_integrations),
        );
        let synthed_names: std::collections::HashSet<String> =
            synthed.iter().map(|t| t.name().to_string()).collect();
        let synthed_specs: Vec<Arc<crate::openhuman::tools::ToolSpec>> =
            synthed.iter().map(|t| Arc::new(t.spec())).collect();

        // Skip mutation when neither the previous nor the next synthesis
        // produced any names — saves work on agents without dynamic
        // delegation. `synthesized_tools` is already empty in that state, so
        // there is nothing to publish either.
        if self.synthesized_tool_names.is_empty() && synthed_names.is_empty() {
            return;
        }

        // Mask of the previous synthesis — the names whose `tool_specs` are
        // currently live (this set is kept in lock-step with `tool_specs`).
        let old_synth = std::mem::take(&mut self.synthesized_tool_names);

        // `tool_specs` are plain data and therefore cloneable. Drop exactly the
        // previous synthesised spec set, then append the fresh one.
        {
            let specs_vec = Arc::make_mut(&mut self.tool_specs);
            specs_vec.retain(|s| !old_synth.contains(&s.name));
            specs_vec.extend(synthed_specs);
        }

        // The executable instances are replaced wholesale. `synthed` already IS
        // the complete new set — `collect_orchestrator_tools` rebuilds every
        // delegate from the current connection set — so there is nothing to
        // retain and no mask to apply: assigning a fresh `Arc` drops exactly
        // the previous synthesis and nothing else.
        //
        // This is the step that used to be conditional on `Arc::get_mut`
        // succeeding against `self.tools`. It no longer touches `self.tools` at
        // all, so a concurrent reader cannot block it, and the specs above and
        // the instances here can never drift apart again (#6145).
        // Readers still holding the previous `Arc` keep a coherent set for the
        // rest of their turn; those instances are freed when the last one goes.
        let previous_instances = self.synthesized_tools.len();
        self.synthesized_tools = Arc::new(synthed);
        // The pack tool's handle holds a `Weak` into the allocation that was
        // just replaced. Without this re-bind it stops upgrading once the last
        // reader of the old set goes, and every packed delegate — `do_crypto`,
        // `make_presentation`, `create_image`, … — answers "no tool in skill"
        // instead of running: withheld from the wire and unreachable through
        // the route that replaced it.
        crate::openhuman::tools::toolpacks::bind_synthesized_pack_registry(
            &self.tools,
            &self.synthesized_tools,
        );

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
            // The synthesis above re-adds delegate names wholesale, including
            // any that belong to a tool pack — so re-apply the withholding here
            // or a packed `delegate_*` tool would reappear on the wire on the
            // first Composio reconcile, silently undoing the compression.
            let agent_id = self.agent_definition_name.clone();
            crate::openhuman::tools::toolpacks::strip_packed_from_visible(
                &mut self.visible_tool_names,
                &agent_id,
            );
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

        // Specs and instances reconciled to the same set in the same pass, so
        // the name mask tracks that set unconditionally.
        self.synthesized_tool_names = synthed_names.clone();

        log::info!(
            "[agent] refresh_delegation_tools: reconciled delegation surface for agent '{}' (display='{}'); now {} synthesised tool name(s); added={:?} removed={:?} superseded_instances={}",
            self.agent_definition_id,
            self.agent_definition_name,
            synthed_names.len(),
            added,
            removed,
            previous_instances
        );
    }
}
