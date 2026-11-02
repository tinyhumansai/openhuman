use super::*;

impl OpenHumanSessionHost {
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
        // Durable `Hidden` members leave the wire here; the deferred split
        // over both sets is the shared recompute below.
        let _ = tinytools::strip_deferred_from_visible(
            &mut self.visible_tool_names,
            self.tools.as_slice(),
        );
        self.recompute_deferred_tool_names();
    }

    /// Re-derive [`Self::deferred_tool_names`] from the current durable and
    /// synthesised sets and take those names off the advertised set. Called
    /// wherever either set changes; a no-op for a belt that never opted into
    /// discovery.
    pub(in crate::agent::session_host) fn recompute_deferred_tool_names(&mut self) {
        self.visible_tool_names
            .extend(self.permanent_tool_names.iter().cloned());
        if !self.discovery_enabled {
            self.deferred_tool_names.clear();
            return;
        }
        let mut deferred = tinytools::deferred_set(
            self.tools.as_slice(),
            self.synthesized_tools.as_slice(),
            &self.requested_deferred_tools,
        );
        deferred.retain(|name| !self.permanent_tool_names.contains(name));
        self.visible_tool_names
            .retain(|name| !deferred.contains(name));
        self.deferred_tool_names = deferred;
    }

    /// The names the harness registers for a turn: the advertised set plus
    /// the deferred set the `tool_search` bridge can reach. The empty
    /// wildcard sentinel stays empty — "no filter" already admits the
    /// deferred tools.
    pub(in crate::agent::session_host) fn reachable_tool_names(
        &self,
    ) -> std::collections::HashSet<String> {
        if self.visible_tool_names.is_empty() {
            return std::collections::HashSet::new();
        }
        self.visible_tool_names
            .iter()
            .chain(self.deferred_tool_names.iter())
            .cloned()
            .collect()
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
        let reachable = self.reachable_tool_names();
        let mut session = tinytools::ToolPolicyEngine::build_session_from_refs(
            &self.agent_definition_name,
            &self.event_channel,
            "session",
            &self.config.channel_permissions,
            &all_tools,
            &reachable,
        );
        // Same narrowing as the builder, re-applied on every rebuild so a
        // delegation refresh cannot reopen a closed pack (#6302).
        crate::tools::toolpacks::close_handed_off_packs(
            &mut session,
            &self.agent_definition_name,
            &all_tools,
        );
        self.tool_policy_session = session;
        let visible_specs = super::super::super::builder::visible_tool_specs_for_policy(
            self.tool_specs.as_slice(),
            &self.visible_tool_names,
            &self.tool_policy_session,
        );
        self.visible_tool_specs =
            Arc::new(tinyagents_runtime::dedup_visible_tool_specs(visible_specs));
    }
}
