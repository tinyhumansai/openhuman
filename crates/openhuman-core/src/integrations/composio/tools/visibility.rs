//! Curated-catalog / user-scope visibility gating shared by the Composio
//! agent tools: [`resolve_action_scope`] / [`action_mutates_external_state`]
//! (used by [`super::super::action_tool`]'s sandbox and approval gates),
//! [`evaluate_tool_visibility`] / [`filter_list_tools_response`] (used by
//! `composio_list_tools`), and the small markdown-rendering helpers
//! `composio_list_tools` uses for its compact output.

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;

use crate::config::Config;

use super::super::contract::{
    catalog_for_toolkit, classify_unknown, find_curated, toolkit_from_slug, ToolScope,
    UserScopePref,
};
use super::super::ops::load_user_scope_pref;

/// Decision returned by [`evaluate_tool_visibility`].
pub(super) enum ToolDecision {
    /// Action is curated for this toolkit and user scope allows it.
    Allow,
    /// Action exists in the curated list but the user's scope blocks
    /// it. `scope` is the curated classification.
    BlockedByScope { scope: ToolScope },
    /// Action is not in the toolkit's curated whitelist (and the
    /// toolkit has one). Hidden / rejected.
    NotCurated,
    /// Toolkit has no curated catalog — pass through, but still gate by
    /// the user scope using the [`classify_unknown`] heuristic.
    PassthroughCheckScope { scope: ToolScope },
}

/// Resolve a Composio action slug to its [`ToolScope`] classification.
///
/// Prefers the toolkit's curated catalog when available (most accurate
/// — curated entries are hand-classified) and falls back to the
/// [`classify_unknown`] heuristic for un-curated toolkits. Unparseable
/// slugs default to `Write` so the sandbox gate errs on the side of
/// blocking rather than letting a potentially-mutating action slip
/// through uncategorised.
pub(crate) async fn resolve_action_scope(slug: &str) -> ToolScope {
    resolve_action_scope_sync(slug)
}

/// Synchronous core used by policy hooks that cannot await.
fn resolve_action_scope_sync(slug: &str) -> ToolScope {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        return ToolScope::Write;
    };
    let catalog = catalog_for_toolkit(&toolkit);
    if let Some(cat) = catalog {
        if let Some(entry) = find_curated(cat, slug) {
            return entry.scope;
        }
    }
    classify_unknown(slug)
}

/// Whether an action must pass through the human approval gate.
pub(crate) fn action_mutates_external_state(slug: &str) -> bool {
    matches!(
        resolve_action_scope_sync(slug),
        ToolScope::Write | ToolScope::Admin
    )
}

/// Decide whether a Composio action slug should be visible / executable
/// for the current user, given the registered provider's curated list
/// (if any) and the user's stored scope preference.
pub(super) async fn evaluate_tool_visibility(config: &Config, slug: &str) -> ToolDecision {
    let Some(toolkit) = toolkit_from_slug(slug) else {
        // Unparseable slug — let the backend return its own error.
        return ToolDecision::Allow;
    };
    let pref = load_user_scope_pref(config, &toolkit).await;
    // The catalog covers every catalogued toolkit directly now — the
    // engine's `get_provider(toolkit).curated_tools()` hop this used to
    // prefer was pure indirection, verified identical to `catalog_for_toolkit`
    // for every toolkit that had a native provider.
    let catalog = catalog_for_toolkit(&toolkit);
    match catalog {
        Some(catalog) => match find_curated(catalog, slug) {
            Some(curated) if pref.allows(curated.scope) => ToolDecision::Allow,
            Some(curated) => ToolDecision::BlockedByScope {
                scope: curated.scope,
            },
            None => ToolDecision::NotCurated,
        },
        None => {
            let scope = classify_unknown(slug);
            if pref.allows(scope) {
                ToolDecision::PassthroughCheckScope { scope }
            } else {
                ToolDecision::BlockedByScope { scope }
            }
        }
    }
}

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

/// Filter a freshly-fetched [`super::super::types::ComposioToolsResponse`] in
/// place: drop tools that aren't curated for their toolkit and tools
/// whose scope is disabled in the user's pref.
pub(super) async fn filter_list_tools_response(
    config: &Config,
    resp: &mut super::super::types::ComposioToolsResponse,
) {
    let before = resp.tools.len();
    // Compute keep/drop decisions sequentially (the await means we
    // can't fold this into a single sync `retain` closure). Then zip
    // each tool with its decision and collect the survivors — clearer
    // than juggling a parallel index alongside `Vec::retain`.
    let mut keep: Vec<bool> = Vec::with_capacity(before);
    for t in &resp.tools {
        let decision = evaluate_tool_visibility(config, &t.function.name).await;
        keep.push(matches!(
            decision,
            ToolDecision::Allow | ToolDecision::PassthroughCheckScope { .. }
        ));
    }
    let drained: Vec<_> = resp.tools.drain(..).collect();
    resp.tools = drained
        .into_iter()
        .zip(keep)
        .filter_map(|(tool, keep_it)| if keep_it { Some(tool) } else { None })
        .collect();
    let after = resp.tools.len();
    if after != before {
        tracing::debug!(
            before,
            after,
            dropped = before - after,
            "[composio][scopes] composio_list_tools filtered"
        );
    }
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

// `execute_direct` was previously defined locally here; it now lives
// in `super::super::client::direct_execute` so the ops.rs RPC handler and the
// agent-tool path share a single direct-mode envelope reshaper.
// See `direct_execute`'s rustdoc for the v3 → ComposioExecuteResponse
// translation contract.

/// Format a user-facing error message for a scope-blocked execution.
///
/// Embeds the unlock path in the error itself so the agent reads the
/// instruction straight off the tool response — same policy-in-data
/// approach as the `gated_tools` surface. Only ONE path: the user
/// toggles the scope in the Connections UI. The agent has no tool to
/// flip scopes (see the note above the removed `ComposioEnableScopeTool`
/// for why) — it can only describe the gate and point at the UI.
pub(super) fn scope_error_message(slug: &str, scope: ToolScope, pref: UserScopePref) -> String {
    let toolkit = toolkit_from_slug(slug).unwrap_or_default();
    let scope_str = scope.as_str();
    format!(
        "composio_execute: action `{slug}` is classified `{scope_str}` and is \
         disabled in the user's current scope preferences for `{toolkit}` \
         (read={}, write={}, admin={}). Tell the user this action requires the \
         `{scope_str}` scope and they can enable it themselves in \
         **Connections → {toolkit} → {scope_str}**. Do not claim you can flip \
         it — you cannot.",
        pref.read, pref.write, pref.admin,
    )
}
