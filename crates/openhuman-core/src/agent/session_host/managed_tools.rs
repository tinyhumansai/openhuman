//! Refresh only the attached-tool catalogue; keep the host prefix frozen.
use std::{collections::HashSet, sync::Arc};
use tinyagents_runtime::PrefixSnapshot;
use tinyinference_llm::message::Message;
use tinytools::ToolSpec;
const MARKER: &str = "<openhuman-permanent-tools>\n";
pub(super) fn refresh_prefix(
    prefix: &PrefixSnapshot,
    specs: &[Arc<ToolSpec>],
    names: &HashSet<String>,
) -> Option<PrefixSnapshot> {
    if names.is_empty() {
        return None;
    }
    let mut messages: Vec<Message> = prefix
        .messages()
        .iter()
        .filter(|message| !message.text().starts_with(MARKER))
        .cloned()
        .collect();
    let mut entries: Vec<&ToolSpec> = specs
        .iter()
        .filter(|spec| names.contains(&spec.name))
        .map(AsRef::as_ref)
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let catalogue = entries
        .iter()
        .map(|spec| {
            format!(
                "{}: {}\nParameters: {}",
                spec.name, spec.description, spec.parameters
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    messages.push(Message::system(format!(
        "{MARKER}{catalogue}\n</openhuman-permanent-tools>"
    )));
    if messages == prefix.messages() {
        None
    } else {
        Some(PrefixSnapshot::new(messages).refreshing())
    }
}

/// Check before deduplication, preserving the existing synthesized executors
/// and schemas if a new permanent source would shadow them.
pub(super) fn reject_synthesized_collisions(
    names: &HashSet<String>,
    synthesized: &[Box<dyn tinytools::Tool>],
) -> anyhow::Result<()> {
    if let Some(tool) = synthesized.iter().find(|tool| names.contains(tool.name())) {
        anyhow::bail!(
            "permanent tool name collision with synthesized tool: {}",
            tool.name()
        );
    }
    Ok(())
}
