//! Refresh the executable, declared and policy tool surfaces at a turn boundary.

use super::*;

impl OpenHumanTurnPrelude {
    /// Rebuild every delegation-dependent tool view from the current cached
    /// integration set, replacing rather than appending. A revoked delegate is
    /// removed from the executable source, schema, and policy together before
    /// this request is prepared.
    pub(super) fn refresh_delegation_tool_surface(&self) -> anyhow::Result<()> {
        use crate::agent::harness::definition::AgentDefinitionRegistry;
        use crate::tools::agent_policy::ToolPolicyEngine;
        use crate::tools::orchestrator_tools::collect_orchestrator_tools;

        let Some(registry) = AgentDefinitionRegistry::current() else {
            return Ok(());
        };
        let Some(definition) = self.session_definition(&registry) else {
            return Ok(());
        };
        let has_recorded_tool_snapshot = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recorded_tool_snapshot_adopted;
        if definition.subagents.is_empty() && !has_recorded_tool_snapshot {
            return Ok(());
        }
        let (integrations, integrations_are_authoritative, recorded_tool_snapshot_adopted) = {
            let mutable = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                mutable.connected_integrations.clone(),
                mutable.connected_integrations_authoritative,
                mutable.recorded_tool_snapshot_adopted,
            )
        };
        #[cfg(feature = "mcp")]
        let mcp_tools = self.collect_mcp_search_tools();
        let mut surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut collected = collect_orchestrator_tools(&definition, &registry, &integrations);
        #[cfg(feature = "mcp")]
        collected.extend(mcp_tools);
        let rebuilt = self.rebuilt_recorded_tools(
            &definition,
            &surface.tools,
            &collected,
            &integrations,
            integrations_are_authoritative,
        );
        collected.extend(rebuilt);
        super::super::managed_tools::reject_synthesized_collisions(
            &surface.permanent_tool_names,
            &collected,
        )?;
        let synthesized =
            super::super::builder::drop_synthesized_name_collisions(&surface.tools, collected);
        let synthesized_names = synthesized
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<std::collections::HashSet<_>>();
        let previous_synthesized = std::mem::replace(
            &mut surface.synthesized_tool_names,
            synthesized_names.clone(),
        );
        let auto_include_new_synthesized_tools = surface.auto_include_new_synthesized_tools;
        reconcile_synthesized_visibility(
            &mut surface.visible_tool_names,
            &previous_synthesized,
            &synthesized_names,
            auto_include_new_synthesized_tools,
        );
        if recorded_tool_snapshot_adopted {
            surface.visible_tool_names.extend(
                self.mutable
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .recorded_repl_tools
                    .iter()
                    .map(|spec| spec.name.clone()),
            );
        }
        // Preserve permanently attached tools when re-deriving the surface.
        permanent::refresh_visibility(&mut surface, &synthesized);

        let mut specs = surface
            .durable_tool_specs
            .iter()
            .cloned()
            .chain(synthesized.iter().map(|tool| Arc::new(tool.spec())))
            .collect::<Vec<_>>();
        {
            let recorded = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .recorded_repl_tools
                .clone();
            let recorded_names = recorded
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<std::collections::HashSet<_>>();
            if recorded_tool_snapshot_adopted {
                specs.retain(|spec| {
                    !crate::inference::tokenjuice::is_repl_tool(&spec.name)
                        || recorded_names.contains(spec.name.as_str())
                });
                surface.visible_tool_names.retain(|name| {
                    !crate::inference::tokenjuice::is_repl_tool(name)
                        || recorded_names.contains(name.as_str())
                });
            }
            let recorded = recorded
                .iter()
                .map(|spec| (spec.name.as_str(), spec))
                .collect::<std::collections::HashMap<_, _>>();
            for spec in &mut specs {
                if let Some(frozen) = recorded.get(spec.name.as_str()) {
                    *spec = Arc::new((*frozen).clone());
                }
            }
        }
        let synthesized_tools = Arc::new(synthesized);
        crate::tools::toolpacks::bind_synthesized_pack_registry(&surface.tools, &synthesized_tools);
        let all_tools = surface
            .tools
            .iter()
            .chain(synthesized_tools.iter())
            .map(|tool| tool.as_ref())
            .collect::<Vec<_>>();
        // Advertised plus deferred, like the session host: a deferred tool
        // outside the set would be `HideFromPrompt`, which the direct-call
        // gate refuses.
        let reachable: std::collections::HashSet<String> = if surface.visible_tool_names.is_empty()
        {
            std::collections::HashSet::new()
        } else {
            surface
                .visible_tool_names
                .iter()
                .chain(surface.deferred_tool_names.iter())
                .cloned()
                .collect()
        };
        let mut policy = ToolPolicyEngine::build_session_from_refs(
            &surface.agent_definition_name,
            &surface.event_channel,
            "session",
            &self.config.channel_permissions,
            &all_tools,
            &reachable,
        );
        crate::tools::toolpacks::close_handed_off_packs(
            &mut policy,
            &surface.agent_definition_name,
            &all_tools,
        );
        let visible = super::super::builder::dedup_visible_tool_specs(
            super::super::builder::visible_tool_specs_for_policy(
                &specs,
                &surface.visible_tool_names,
                &policy,
            ),
        );
        surface.tool_specs = Arc::new(specs);
        surface.synthesized_tools = synthesized_tools;
        surface.visible_tool_specs = Arc::new(visible);
        surface.tool_policy_session = policy;
        Ok(())
    }
}
