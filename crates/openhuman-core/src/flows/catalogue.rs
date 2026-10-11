//! Saved Flows automations, as entries in the skill catalogue.
//!
//! A user asking "what can this thing already do for me" does not distinguish a
//! SKILL.md bundle from a saved tinyflows graph, and until now the catalogue
//! did. The `## Installed Skills` section listed only bundles and carried ~200
//! bytes of caveat teaching the model that the list it was reading deliberately
//! omitted half the answer — *"it only knows about entries in this list, not
//! Flows automations — do not call it with a Flows `workflow_id`, it will
//! error"* — plus a pointer to a different tool for the omitted half. Prose
//! that exists to explain a gap is usually cheaper to spend on closing it.
//!
//! So a flow becomes a [`Workflow`] with [`WorkflowScope::Flow`], and the two
//! consumers that answer "what is installed" — the orchestrator's catalogue
//! section and `skill_search` — see one list.
//!
//! # What a Flow entry is not
//!
//! It is a **listing**, not a bundle. Every other scope is a directory the
//! skill scanner walked; this is a row in `flows.db`. There is no `SKILL.md`,
//! so `location` is `None` and `resources` is empty, and the tools that read
//! those (`describe_workflow`, `read_workflow_resource`) must say so by name
//! rather than failing on a missing file. That is the whole reason
//! `WorkflowScope::Flow` is a distinct variant instead of these being smuggled
//! in as `User` skills: the difference is real, and a consumer that needs to
//! know can ask.
//!
//! # Descriptions: the graph's shape
//!
//! Saved flows carry the name and graph metadata supplied by the shared catalog
//! crate, but no separate catalogue description. The summary below therefore
//! describes only the graph's shape, without inventing a purpose from node
//! internals.

use crate::config::Config;
use {tinyskills::Skill as Workflow, tinyskills::SkillScope as WorkflowScope};

/// How many flows the catalogue will surface.
///
/// Flows are cheap to create and a heavy user can accumulate many, while this
/// list is rendered into a system prompt that is frozen for the whole session.
/// The cap is a ceiling on a per-turn cost that would otherwise grow silently
/// with the contents of a database; every flow past it stays runnable by name,
/// only its catalogue line is gone. Mirrors `MAX_LISTED_SKILLS` in the
/// orchestrator prompt, which caps the same section from the other side.
pub const MAX_CATALOGUE_FLOWS: usize = 20;

/// Every saved flow, as catalogue entries.
///
/// Returns an empty vec on any store error rather than propagating: this feeds
/// a prompt section and a search index, and a transient `flows.db` problem
/// should degrade the catalogue, never fail the turn. The error is logged.
pub fn flow_entries(config: &Config) -> Vec<Workflow> {
    let (flows, skipped) = match super::store::list_flows(config) {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!(%error, "[flows][catalogue] could not list flows; catalogue omits them");
            return Vec::new();
        }
    };
    if skipped > 0 {
        // `list_flows` documents that a non-zero `skipped` must be surfaced
        // loudly rather than treated as a reason to fail.
        tracing::warn!(
            skipped,
            "[flows][catalogue] some flow rows could not be decoded and are absent from the catalogue"
        );
    }

    let total = flows.len();
    let mut entries: Vec<Workflow> = flows
        .into_iter()
        // Disabled flows are deliberately listed. A user who switched one off
        // still owns it, and a catalogue that hid it would make the model
        // answer "you have no such automation" to someone looking at it in the
        // UI. The entry says it is paused; the model can offer to enable it.
        .take(MAX_CATALOGUE_FLOWS)
        .map(entry_for)
        .collect();
    if total > MAX_CATALOGUE_FLOWS {
        tracing::debug!(
            total,
            listed = MAX_CATALOGUE_FLOWS,
            "[flows][catalogue] flow list truncated for the prompt catalogue"
        );
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn entry_for(flow: tinyflows_catalog::types::Flow) -> Workflow {
    Workflow {
        name: flow.name.clone(),
        // The flow id, because that is what `run_workflow` / `get_flow` take.
        // A slug of the name would be a second identifier that resolves
        // nowhere.
        dir_name: flow.id.clone(),
        description: describe(&flow),
        scope: WorkflowScope::Flow,
        // No bundle on disk: no manifest to read, no resources to page
        // through. Left explicitly empty so a consumer that reads them gets an
        // honest absence rather than a path that does not exist.
        location: None,
        ..Default::default()
    }
}

/// A structural summary of the flow graph.
fn describe(flow: &tinyflows_catalog::types::Flow) -> String {
    describe_shape(flow)
}

/// A one-line summary of what the graph *is*, for a flow with no description.
///
/// Deliberately structural — trigger, size, paused-ness — because that is all
/// the record carries. See the module docs: inventing a purpose from node
/// internals would read as authoritative and frequently be wrong.
fn describe_shape(flow: &tinyflows_catalog::types::Flow) -> String {
    // Read out of the trigger node's free-form config, which is where the
    // engine keeps it — the same way `tinyflows`' own `trigger_kind` does.
    // A graph with zero or several triggers has no single answer, and
    // validation reports that separately, so this stays quiet.
    let trigger = flow
        .graph
        .trigger()
        .and_then(|node| node.config.get("trigger_kind"))
        .and_then(|value| value.as_str())
        .unwrap_or("manual")
        .to_string();
    let steps = flow
        .graph
        .nodes
        .len()
        // The trigger is not a step the user thinks about.
        .saturating_sub(1);
    let mut out = format!(
        "Saved Flows automation ({trigger} trigger, {steps} step{}).",
        if steps == 1 { "" } else { "s" }
    );
    if !flow.enabled {
        out.push_str(" Currently disabled.");
    }
    out
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
