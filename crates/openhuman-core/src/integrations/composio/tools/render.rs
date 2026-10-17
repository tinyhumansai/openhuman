//! Compact rendering and connected-toolkit shaping for `composio_list_tools`.
//!
//! Output formatting only: which actions the user's scope preference exposes
//! is decided in [`super::scope_gate`] against the connector module's
//! vocabulary.

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;

use super::super::contract::{catalog_for_toolkit, toolkit_from_slug};

/// Drop tools whose toolkit is not in `connected` (case-insensitive).
/// Returns the number of dropped tools so callers can log it.
/// `toolkit_from_slug` already lowercases its result, so the comparison
/// is direct against entries the caller has already lowercased.
pub(super) fn retain_connected_tools(
    resp: &mut super::super::types::ComposioToolsResponse,
    connected: &HashSet<String>,
) -> usize {
    let before = resp.tools.len();
    resp.tools.retain(|t| {
        toolkit_from_slug(&t.function.name)
            .map(|tk| connected.contains(&tk))
            .unwrap_or(false)
    });
    before - resp.tools.len()
}

pub(super) fn normalized_scope_toolkits(
    requested: Option<&[String]>,
    connected: Option<&HashSet<String>>,
) -> Vec<String> {
    let mut out = BTreeSet::new();
    if let Some(requested) = requested {
        for toolkit in requested {
            let normalized = toolkit.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                out.insert(normalized);
            }
        }
    } else if let Some(connected) = connected {
        out.extend(connected.iter().filter(|t| !t.is_empty()).cloned());
    }
    out.into_iter().collect()
}

pub(super) fn uncatalogued_toolkits(toolkits: &[String]) -> Vec<String> {
    toolkits
        .iter()
        .filter(|toolkit| catalog_for_toolkit(toolkit).is_none())
        .cloned()
        .collect()
}

pub(super) fn empty_uncurated_toolkits_message(toolkits: &[String]) -> Option<String> {
    let unsupported = uncatalogued_toolkits(toolkits);
    if unsupported.is_empty() {
        return None;
    }
    let names = unsupported
        .iter()
        .map(|toolkit| format!("`{toolkit}`"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "composio_list_tools: no agent-ready actions are available for toolkit(s) {names}. \
         These integrations can be connected, but OpenHuman does not yet ship curated agent \
         tool catalogs for them. Use a supported toolkit such as Google Drive or Google Sheets \
         for now, or try again after catalog support lands."
    ))
}

/// One-line description: collapse whitespace + truncate.
fn one_line(desc: &str, max_chars: usize) -> String {
    let collapsed: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let snippet: String = collapsed.chars().take(max_chars).collect();
        format!("{snippet}…")
    }
}

/// Pull required + optional top-level argument names from a JSON Schema
/// `parameters` object. Returns `(required, optional)` — both empty when
/// the schema is missing or doesn't follow the expected shape.
fn split_arg_names(parameters: Option<&Value>) -> (Vec<String>, Vec<String>) {
    let Some(params) = parameters.and_then(Value::as_object) else {
        return (Vec::new(), Vec::new());
    };
    let required: Vec<String> = params
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut optional: Vec<String> = params
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();
    optional.retain(|k| !required.contains(k));
    (required, optional)
}

/// Compact markdown rendering of `composio_list_tools` output.
///
/// Drops the full JSON parameter schemas (the main token cost) and keeps
/// only what the agent needs to pick a slug and call `composio_execute`:
/// the slug, a one-line description, and the names of required +
/// optional top-level arguments. Tools are grouped by toolkit prefix.
pub(super) fn render_tools_markdown(resp: &super::super::types::ComposioToolsResponse) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    if resp.tools.is_empty() {
        return "_No composio tools available._".to_string();
    }

    // Group by toolkit slug (lowercase prefix). Use BTreeMap for stable
    // ordering so the agent sees the same shape across calls.
    let mut by_toolkit: BTreeMap<String, Vec<&super::super::types::ComposioToolSchema>> =
        BTreeMap::new();
    for t in &resp.tools {
        let toolkit = toolkit_from_slug(&t.function.name).unwrap_or_else(|| "other".to_string());
        by_toolkit.entry(toolkit).or_default().push(t);
    }

    let mut out = format!(
        "# Composio tools ({} actions across {} toolkit{})\n\n\
         Call `composio_execute` with `tool=<SLUG>` and an `arguments` object \
         matching the listed parameters.\n",
        resp.tools.len(),
        by_toolkit.len(),
        if by_toolkit.len() == 1 { "" } else { "s" },
    );

    for (toolkit, tools) in &by_toolkit {
        let _ = writeln!(out, "\n## {toolkit}");
        for t in tools {
            let desc = t
                .function
                .description
                .as_deref()
                .map(|d| one_line(d, 160))
                .unwrap_or_default();
            let (required, optional) = split_arg_names(t.function.parameters.as_ref());
            let _ = write!(out, "- `{}`", t.function.name);
            if !desc.is_empty() {
                let _ = write!(out, " — {desc}");
            }
            if !required.is_empty() {
                let _ = write!(out, " **req:** {}", required.join(", "));
            }
            if !optional.is_empty() {
                let _ = write!(out, " **opt:** {}", optional.join(", "));
            }
            out.push('\n');
        }
    }
    out
}
