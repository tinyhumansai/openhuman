//! Rebuilds the executors a resumed thread already declared to the model.
//!
//! The tinyagents session records the exact tool declarations each turn was
//! sent with and hands them back on resume (`SessionStateView::recorded_tools`).
//! A new process, though, rebuilds its live tool surface from state that may
//! not be there yet: Composio actions are synthesised from the connected
//! integrations list, which comes from an in-process cache that is empty
//! after a restart and may be unreachable. Without a fallback, a resumed
//! thread whose prompt says "search for the Gmail action" loses every
//! integration action — and with it the `tool_search` bridge — for the turn.
//!
//! MCP tools do not need this: they are rebuilt every turn from tinymcp's
//! persistent tool cache, which survives a restart (see
//! `mcp::registry::action_tool`).
//!
//! This module turns the recorded Composio action declarations back into
//! executable deferred tools, so the tool list a thread was sent never
//! shrinks just because a cache went cold. The live surface stays
//! authoritative for any action it still supplies.

use std::collections::HashSet;

use tinytools::{Tool, ToolSpec};

use crate::integrations::composio::action_tool::ComposioActionTool;

/// Whether `name` is a Composio action slug (`GMAIL_SEND_EMAIL`).
///
/// Composio slugs are upper-case `TOOLKIT_ACTION`; every OpenHuman-owned tool
/// name is lower-case snake case, so the shapes never overlap.
pub(crate) fn is_integration_action_name(name: &str) -> bool {
    let mut parts = name.splitn(2, '_');
    let (Some(toolkit), Some(action)) = (parts.next(), parts.next()) else {
        return false;
    };
    !toolkit.is_empty()
        && !action.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

/// The toolkit slug an action slug belongs to (`GMAIL_SEND_EMAIL` → `gmail`).
/// Only used as the tool's search family.
fn toolkit_of(action: &str) -> String {
    action
        .split('_')
        .next()
        .unwrap_or(action)
        .to_ascii_lowercase()
}

/// The recorded integration action declarations, in recorded order.
pub(crate) fn recorded_integration_actions(recorded: &[ToolSpec]) -> Vec<ToolSpec> {
    recorded
        .iter()
        .filter(|spec| is_integration_action_name(&spec.name))
        .cloned()
        .collect()
}

/// Deferred executors for every recorded action the live set does not
/// already provide. `live` names win: a still-connected integration keeps
/// its freshly fetched declaration.
pub(crate) fn rehydrate_integration_actions(
    recorded: &[ToolSpec],
    live: &[Box<dyn Tool>],
    integrations: &[crate::integrations::ConnectedIntegration],
    integrations_are_authoritative: bool,
) -> Vec<Box<dyn Tool>> {
    let live_names: HashSet<&str> = live.iter().map(|tool| tool.name()).collect();
    let mut seen = HashSet::new();
    recorded
        .iter()
        // Only Composio's upper-case `TOOLKIT_ACTION` declarations may be
        // reconstructed. A recorded OpenHuman tool such as `web_fetch` is
        // historical prompt state, not an integration action.
        .filter(|spec| is_integration_action_name(&spec.name))
        // Transcript declarations are historical state, never authorization.
        // Do not make a deferred executor available until a current
        // authoritative integration snapshot permits its toolkit and action.
        .filter(|spec| {
            integrations_are_authoritative
                && integrations.iter().any(|integration| {
                    integration.connected
                        && integration
                            .toolkit
                            .eq_ignore_ascii_case(&toolkit_of(&spec.name))
                        && !integration
                            .gated_tools
                            .iter()
                            .any(|gated| gated.name == spec.name)
                })
        })
        .filter(|spec| !live_names.contains(spec.name.as_str()))
        .filter(|spec| seen.insert(spec.name.clone()))
        .map(|spec| {
            Box::new(ComposioActionTool::deferred(
                &toolkit_of(&spec.name),
                spec.name.clone(),
                spec.description.clone(),
                Some(spec.parameters.clone()),
            )) as Box<dyn Tool>
        })
        .collect()
}

/// The recorded TinySearch role tools (`web_search_tool`, `web_answer_tool`,
/// `web_contents_tool`), in recorded order.
pub(crate) fn recorded_search_tools(recorded: &[ToolSpec]) -> Vec<ToolSpec> {
    recorded
        .iter()
        .filter(|spec| is_search_role_tool(&spec.name))
        .cloned()
        .collect()
}

fn is_search_role_tool(name: &str) -> bool {
    tinysearch_bus::role_for_tool(name).is_some()
}

/// Executors for recorded search role tools the live surface no longer
/// builds (the user signed out, or turned a provider off). Each keeps its
/// recorded declaration and resolves the live config per call, so a call
/// answers with the actionable "no usable provider" error instead of an
/// unknown-tool failure.
#[cfg(feature = "modules")]
pub(crate) fn rehydrate_search_tools(
    recorded: &[ToolSpec],
    live: &[&[Box<dyn Tool>]],
    agent_id: &str,
) -> Vec<Box<dyn Tool>> {
    let live_names: HashSet<&str> = live
        .iter()
        .flat_map(|tools| tools.iter().map(|tool| tool.name()))
        .collect();
    let mut seen = HashSet::new();
    let rebuilt: Vec<Box<dyn Tool>> = recorded
        .iter()
        .filter(|spec| is_search_role_tool(&spec.name))
        .filter(|spec| !live_names.contains(spec.name.as_str()))
        .filter(|spec| seen.insert(spec.name.clone()))
        .map(|spec| {
            Box::new(crate::search::TinySearchTool::recorded(
                tinysearch_bus::ToolSpec {
                    name: spec.name.clone(),
                    description: spec.description.clone(),
                    parameters: spec.parameters.clone(),
                },
            )) as Box<dyn Tool>
        })
        .collect();
    if !rebuilt.is_empty() {
        log::info!(
            "[session] rebuilt {} recorded search tool(s) the live surface did not supply agent={agent_id}",
            rebuilt.len()
        );
    }
    rebuilt
}

#[cfg(test)]
#[path = "recorded_tools_tests.rs"]
mod tests;
