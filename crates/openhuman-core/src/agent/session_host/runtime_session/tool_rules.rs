//! Tool rules for a session's turns (`crate::tools::rules`).
//!
//! A session composes its rule layers once — the operator's `[tool_rules]`
//! and its agent definition's layer — and every turn evaluates them in that
//! session's context (channel, agent), installed on
//! `OpenHumanRunContext::tool_rules` for the harness gate.

use std::sync::Arc;

use super::{OpenHumanSessionHost, OpenHumanTurnPrelude, OpenHumanTurnToolSurface};
use crate::agent::tinyagents::host::OpenHumanRunContext;
use tinyagents_harness::tool::ToolRulePolicy;
use tinyagents_runtime::SessionTurnRequest;
use tinytools::{Surface, ToolSubject};

impl OpenHumanSessionHost {
    /// The session's rule layers, from its config and resolved definition.
    pub(super) fn session_tool_rules(&self) -> Arc<tinytools::ToolRuleSet> {
        Arc::new(crate::tools::rules::session_rule_set(
            self.runtime_config.as_deref(),
            self.resolved_definition().as_deref(),
        ))
    }
}

impl OpenHumanTurnPrelude {
    /// Names the session's rules keep off a listing — the catalogue for a
    /// direct tool, search for a deferred one — evaluated in the session's
    /// channel/agent context. The host renders its own tool catalogue into
    /// the system prompt, so it must drop these exactly as the harness drops
    /// their schemas; a hidden tool stays callable, it is just not listed.
    pub(super) fn rule_withheld_tools(
        &self,
        surface: &OpenHumanTurnToolSurface,
    ) -> std::collections::HashSet<String> {
        if self.tool_rules.is_permissive() {
            return std::collections::HashSet::new();
        }
        let context = crate::tools::rules::rule_context(
            Some(&surface.event_channel),
            Some(&self.agent_definition_id),
            None,
        );
        surface
            .tools
            .iter()
            .chain(surface.synthesized_tools.iter())
            .filter(|tool| {
                let surface_kind = if surface.deferred_tool_names.contains(tool.name()) {
                    Surface::Search
                } else {
                    Surface::Catalog
                };
                !self
                    .tool_rules
                    .visible(&ToolSubject::of(tool.as_ref()), &context, surface_kind)
            })
            .map(|tool| tool.name().to_string())
            .collect()
    }

    /// Records the user text a turn begins with, for its user-side effects.
    pub(super) fn begin_user_effects(
        &self,
        request: &SessionTurnRequest,
        run_context: &OpenHumanRunContext,
    ) {
        let user_text = run_context
            .dispatch_ctx()
            .turn_origin_or_scoped()
            .is_some_and(|origin| origin.is_user_authored())
            .then(|| request.input.text());
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_user_text = user_text;
    }

    /// This turn's rule policy: the session's layers in the turn's context.
    /// `None` when the session's rules restrict nothing.
    pub(super) fn turn_tool_rules(
        &self,
        channel: &str,
        run_context: &OpenHumanRunContext,
    ) -> Option<Arc<ToolRulePolicy>> {
        if self.tool_rules.is_permissive() {
            return None;
        }
        // The session's own context only (channel, agent), never the turn's
        // origin: the host-rendered catalogue is cached for the session, and
        // it must list exactly what this policy admits on every turn.
        let _ = run_context;
        let context =
            crate::tools::rules::rule_context(Some(channel), Some(&self.agent_definition_id), None);
        Some(Arc::new(crate::tools::rules::turn_rule_policy(
            self.tool_rules.clone(),
            context,
        )))
    }
}

#[cfg(test)]
#[path = "tool_rules_tests.rs"]
mod tests;
