//! Tool execution and Composio delegation refresh.

use super::super::types::OpenHumanSessionHost;

use std::sync::Arc;

impl OpenHumanSessionHost {
    // ─────────────────────────────────────────────────────────────────
    // Sub-agent context snapshots
    // ─────────────────────────────────────────────────────────────────

    /// Fetches the user's active Composio connections and populates
    /// `self.connected_integrations` so the system prompt can surface them.
    ///
    /// Delegates to the shared [`crate::integrations::composio::fetch_connected_integrations`]
    /// which is the single source of truth for integration discovery.
    ///
    /// **No session-scoped Composio client is cached on the agent any
    /// more (#1710 Wave 2)**. Every downstream caller that needs to
    /// dispatch a Composio action now resolves a fresh client via
    /// [`crate::integrations::composio::client::resolve_composio_route`]
    /// at call time so the live `composio.mode` toggle is honoured
    /// without rebuilding the session — see `ComposioActionTool`,
    /// `ProviderContext::execute`, the 5 migrated agent tools in
    /// `composio/tools.rs`, and the spawn-time per-action tool build
    /// path in `subagent_host/ops.rs`.
    pub async fn fetch_connected_integrations(&mut self) {
        let config = match self.runtime_config.clone() {
            Some(config) => config,
            None => match crate::config::Config::load_or_init().await {
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
            crate::integrations::composio::fetch_connected_integrations(&config).await;
        self.connected_integrations_initialized = true;
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
    /// [`crate::integrations::composio::cached_active_integrations`]).
    ///
    /// **Concurrency**: this cannot fail on a shared session. The synthesised
    /// instances live in their own [`OpenHumanSessionHost::synthesized_tools`] `Arc`, which is
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
        use crate::agent::harness::host_definition::AgentDefinitionRegistry;
        use crate::tools::orchestrator_tools::collect_orchestrator_tools;

        let Some(reg) = AgentDefinitionRegistry::current() else {
            // No registry — there's nothing we can do until the
            // registry is initialised. The agent's surface stays at
            // whatever the builder produced.
            return;
        };
        let Some(def) = self.resolved_definition() else {
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
            collect_orchestrator_tools(&def, &reg, &self.connected_integrations),
        );
        let synthed_names: std::collections::HashSet<String> =
            synthed.iter().map(|t| t.name().to_string()).collect();
        let synthed_specs: Vec<Arc<tinytools::ToolSpec>> =
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
        crate::tools::toolpacks::bind_synthesized_pack_registry(
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
            crate::tools::toolpacks::strip_packed_from_visible(
                &mut self.visible_tool_names,
                &agent_id,
            );
        }
        // The synthesis above can carry `Deferred` entries (per-action
        // integration tools for a newly connected toolkit); keep them off the
        // wire and in the searchable set, exactly as the build did.
        self.recompute_deferred_tool_names();

        // Rebuild the visible-spec cache from the new tool_specs so the
        // next provider call carries the reconciled schema. Dedup
        // afterward so a delegate synthesised here (e.g.
        // `delegate_name = "plan"`) doesn't collide with a
        // same-named skill tool on the wire — Anthropic 400s on dup
        // tool names where OpenHuman's backend silently accepts.
        self.rebuild_tool_policy_session();
        self.sync_runtime_tool_surface();

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
