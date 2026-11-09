use std::collections::HashMap;

use tinyagents_session::transcript::{session_stem, SessionRef};

/// The agent definitions a stem may belong to, by the component their ids
/// encode to in a stem.
pub(crate) fn known_agents() -> HashMap<String, String> {
    let mut ids: Vec<String> =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
            .list()
            .iter()
            .map(|def| def.id.clone())
            .collect();
    if let Some(current) =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
    {
        ids.extend(current.list().iter().map(|def| def.id.clone()));
    }
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .filter_map(|id| agent_component(&id).map(|component| (component, id)))
        .collect()
}

/// The text a stem carries for agent `id`: `session_stem` of a probe session,
/// without its session key.
fn agent_component(id: &str) -> Option<String> {
    let stem = session_stem(&SessionRef::scoped("probe", id));
    stem.split_once('.').map(|(_, agent)| agent.to_owned())
}

/// The agent whose component `stem` carries, if it is one of `known`.
pub(crate) fn agent_of(stem: &str, known: &HashMap<String, String>) -> Option<String> {
    // Sub-agent transcripts (`parent__child`) belong to their root's agent.
    let root = stem.split("__").next().unwrap_or(stem);
    let mut parts: Vec<&str> = root.split('.').collect();
    if parts.last().is_some_and(|last| {
        last.strip_prefix('g')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    }) {
        parts.pop();
    }
    if parts.len() < 2 {
        return None;
    }
    known.get(*parts.last()?).cloned()
}
