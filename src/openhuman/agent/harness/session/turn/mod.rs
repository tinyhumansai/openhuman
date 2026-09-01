//! Turn lifecycle: running a single interaction, executing tools, and
//! wiring context stats + the sub-agent harness around them.

mod context;
mod core;
mod graph;
mod session_io;
mod tools;

use crate::openhuman::agent::dispatcher::ParsedToolCall;

use std::borrow::Cow;

/// Built-in direct tools that the orchestrator should call by name, not
/// wrapped in `run_workflow`.
const DIRECT_TOOL_NAMES: &[&str] = &[
    "cron_add",
    "cron_list",
    "cron_remove",
    "cron_update",
    "cron_run",
    "cron_runs",
    "current_time",
];

/// Recovery shim for legacy/wrong-model calls of the form:
/// `run_workflow({workflow_id: "<built-in tool>", inputs: {...}})` (or the
/// pre-rename `run_skill({skill_id: ...})`).
///
/// When this pattern appears, rewrite it into a direct tool call so the turn
/// can proceed without a manual retry.
pub(super) fn normalize_tool_call<'a>(call: &'a ParsedToolCall) -> Cow<'a, ParsedToolCall> {
    if call.name != "run_workflow" && call.name != "run_skill" {
        return Cow::Borrowed(call);
    }
    // Accept either the current `workflow_id` arg or the legacy `skill_id`.
    let Some(target) = call
        .arguments
        .get("workflow_id")
        .or_else(|| call.arguments.get("skill_id"))
        .and_then(|v| v.as_str())
    else {
        return Cow::Borrowed(call);
    };
    if !DIRECT_TOOL_NAMES.contains(&target) {
        return Cow::Borrowed(call);
    }
    let Some(inputs) = call.arguments.get("inputs").and_then(|v| v.as_object()) else {
        return Cow::Borrowed(call);
    };

    log::warn!(
        "[agent_loop] rewrote legacy {}->{} call into direct tool invocation",
        call.name,
        target
    );
    let skill_id = target;
    Cow::Owned(ParsedToolCall {
        name: skill_id.to_string(),
        arguments: serde_json::Value::Object(inputs.clone()),
        tool_call_id: call.tool_call_id.clone(),
    })
}

/// Compute the one-shot mid-session connect announcement.
///
/// Given the toolkit slugs currently connected and the set of slugs already
/// announced to the model this session, returns a natural-language note for
/// any genuinely-new slugs (and records them in `announced` so they are never
/// re-announced). Returns `None` when nothing new connected.
///
/// Kept as a free function (no `&self`) so the delta logic is unit-testable
/// without standing up a full `Agent` — see `turn_tests.rs`.
/// Returns the toolkit slugs in `connected` that have not yet been announced
/// this session, marking them announced. Empty when nothing is new.
pub(super) fn newly_connected_slugs(
    connected: &[String],
    announced: &mut std::collections::HashSet<String>,
) -> Vec<String> {
    let newly: Vec<String> = connected
        .iter()
        .filter(|slug| !announced.contains(*slug))
        .cloned()
        .collect();
    for slug in &newly {
        announced.insert(slug.clone());
    }
    newly
}

/// Render the one-shot user-turn note for a set of freshly-connected slugs.
/// Empty input yields `None`.
pub(super) fn integration_announcement_note(slugs: &[String]) -> Option<String> {
    if slugs.is_empty() {
        return None;
    }
    Some(format!(
        "[integration update] These integration(s) connected during this conversation and are available right now: {}. \
Use delegate_to_integrations_agent with the matching toolkit slug to act on them immediately — do not tell the user to reconnect or restart.",
        slugs.join(", ")
    ))
}

/// Render the one-shot user-turn note for MCP server(s) that connected
/// mid-session. The MCP analogue of [`integration_announcement_note`]: the
/// system-prompt `## Connected MCP Servers` block is frozen at turn 1 (KV-cache
/// prefix), so a server connected mid-conversation is surfaced here instead, on
/// the user turn. Empty input yields `None`.
pub(super) fn mcp_announcement_note(servers: &[String]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    Some(format!(
        "[MCP update] These MCP server(s) connected during this conversation and are available right now: {}. \
Use the use_mcp_server delegate to act on them immediately — do not tell the user to reconnect or restart.",
        servers.join(", ")
    ))
}

/// One-shot note prepended to the next user turn when skills are installed
/// mid-session. Mirrors [`integration_announcement_note`] for the
/// `## Installed Skills` catalogue: tells the model the freshly-installed
/// skills are usable now (via `run_skill`) so it acts instead of claiming
/// they aren't installed from stale context. Returns `None` when nothing is
/// pending. Rides the user turn (not the system prompt) to keep the KV-cache
/// prefix stable.
pub(super) fn skill_announcement_note(skill_ids: &[String]) -> Option<String> {
    if skill_ids.is_empty() {
        return None;
    }
    Some(format!(
        "[skills update] These skill(s) were installed during this conversation and are available right now: {}. \
They are in your `## Installed Skills` list — run one with `run_skill` immediately; do not tell the user to reinstall or restart.",
        skill_ids.join(", ")
    ))
}

/// One-shot note prepended to the next user turn when skills are uninstalled
/// mid-session. Symmetric to [`skill_announcement_note`]: tells the model the
/// listed skills are no longer present and `run_skill` will fail for them, so
/// it does not attempt to invoke them. Rides the user turn (not the system
/// prompt) to keep the KV-cache prefix stable.
pub(super) fn skill_retraction_note(skill_ids: &[String]) -> Option<String> {
    if skill_ids.is_empty() {
        return None;
    }
    Some(format!(
        "[skills retracted] These skill(s) were uninstalled during this conversation and are no longer available: {}. \
Do not attempt to run them with `run_skill` — they have been removed. Tell the user to reinstall if they want to use them again.",
        skill_ids.join(", ")
    ))
}

/// Every namespace's root summary, under user-resolved per-namespace and total
/// caps. The limits are derived from the active
/// [`crate::openhuman::config::schema::agent::MemoryContextWindow`]
/// preset by [`crate::openhuman::config::schema::agent::AgentConfig::resolved_memory_limits`].
///
/// # The shared tree is the driver's now (#5560)
///
/// `memory_subdir == "memory"` goes through
/// `MemoryTree::root_summaries_with_caps` instead of
/// `tree_runtime::store::collect_root_summaries_with_caps`. Same files, same
/// two caps, same `[... truncated]` marker: the embedded driver's member is
/// that function, called with its own `config.workspace_dir`, so the block the
/// prompt renders is unchanged.
///
/// The `memory-<id>` arm stays host-local and untouched. The contract member
/// takes **no path** — deliberately, since a workspace argument would be
/// configuration crossing the bus — and the driver reachable here is bound to
/// the shared subtree, so a dedicated profile's tree still has to be scanned
/// directly by [`collect_profile_tree_root_summaries`].
///
/// # Why `async` is the whole bridge
///
/// The only caller, `turn::context::fetch_learned_context`, is already an
/// `async fn` and awaits four memory reads immediately above this one, so the
/// driver call is awaited rather than bridged. `block_in_place` — the pattern
/// `session::builder::helpers` uses for a genuinely synchronous caller —
/// panics on a current-thread runtime and would buy nothing here.
///
/// An unresolvable binding, a driver with no tree family, or a failed read all
/// yield no summaries. That is what this returned before as well: the engine's
/// scan swallowed its own failures into an empty vector, and the prompt simply
/// carries no memory block.
pub(super) async fn collect_tree_root_summaries(
    workspace_dir: &std::path::Path,
    memory_subdir: &str,
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<crate::openhuman::agent::context::prompt::NamespaceSummary> {
    let rows = if memory_subdir == "memory" {
        driver_tree_root_summaries(per_namespace_cap, total_cap).await
    } else {
        collect_profile_tree_root_summaries(
            &workspace_dir.join(memory_subdir),
            per_namespace_cap,
            total_cap,
        )
    };
    rows.into_iter()
        .map(|(namespace, body, updated_at)| {
            crate::openhuman::agent::context::prompt::NamespaceSummary {
                namespace,
                body,
                updated_at,
            }
        })
        .collect()
}

/// The shared subtree's root summaries, read from the guarded driver.
///
/// Returns the same positional `(namespace, body, updated_at)` triple the
/// engine scan did, so [`collect_tree_root_summaries`]'s mapping is shared with
/// the profile arm rather than written twice.
async fn driver_tree_root_summaries(
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<(String, String, chrono::DateTime<chrono::Utc>)> {
    use crate::openhuman::memory::api::provider::MemoryProvider;

    let guard = match crate::openhuman::memory::ops::guard::active_memory_guard().await {
        Ok(guard) => guard,
        Err(error) => {
            log::debug!("[session::turn] tree root summaries: no bound driver ({error})");
            return Vec::new();
        }
    };
    let Some(tree) = guard.as_tree() else {
        log::debug!(
            "[session::turn] tree root summaries: driver '{}' does not serve Tree",
            guard.driver_id()
        );
        return Vec::new();
    };
    match tree
        .root_summaries_with_caps(per_namespace_cap, total_cap)
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|row| (row.namespace, row.body, row.updated_at))
            .collect(),
        Err(error) => {
            log::warn!("[session::turn] tree root summaries: {error}");
            Vec::new()
        }
    }
}

/// Read summary-tree root summaries from an already-resolved `memory-*` subtree.
/// The engine's compatibility helper hardcodes `<workspace>/memory`; dedicated
/// profiles instead supply `<workspace>/memory-<id>`, so scan that equivalent
/// namespace layout directly rather than falling back to shared memory.
fn collect_profile_tree_root_summaries(
    memory_dir: &std::path::Path,
    per_namespace_cap: usize,
    total_cap: usize,
) -> Vec<(String, String, chrono::DateTime<chrono::Utc>)> {
    let Ok(entries) = std::fs::read_dir(memory_dir.join("namespaces")) else {
        return Vec::new();
    };
    let mut roots: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let namespace = entry.file_name().to_string_lossy().into_owned();
            let raw = std::fs::read_to_string(entry.path().join("tree").join("root.md")).ok()?;
            let node = parse_node_markdown(&raw, &namespace, "root");
            Some((namespace, node))
        })
        .collect();
    roots.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut total_chars = 0usize;
    let mut out = Vec::new();
    for (namespace, node) in roots {
        if total_chars >= total_cap {
            break;
        }
        let body = node.summary.trim();
        if body.is_empty() {
            continue;
        }
        let remaining = total_cap.saturating_sub(total_chars);
        let cap = per_namespace_cap.min(remaining);
        let body_chars = body.chars().count();
        let rendered = if body_chars > cap {
            let mut clipped: String = body.chars().take(cap).collect();
            clipped.push_str("\n\n[... truncated]");
            clipped
        } else {
            body.to_string()
        };
        total_chars += rendered.chars().count();
        out.push((namespace, rendered, node.updated_at));
    }
    out
}

/// Parse one summary-tree node's markdown file into a [`TreeNode`].
///
/// # Why this lives here rather than being called on the engine (#5560)
///
/// This is a verbatim port of `tinycortex`'s
/// `memory::tree::runtime::store::parse_node_markdown`, which
/// [`collect_profile_tree_root_summaries`] used to reach through
/// `parse_node_markdown_pub`. It is brought home rather than routed at the
/// module contract because there is nothing on the contract to route it *at*:
/// `MemoryTree` is namespace-addressed and its summary members answer from the
/// driver's own store, whereas this reads a `root.md` **the driver does not
/// own** — a dedicated profile's `<workspace>/memory-<id>/namespaces/…` tree,
/// which is exactly the subtree the engine's compatibility helper skips
/// because it hardcodes `<workspace>/memory`. A bus round-trip cannot answer a
/// question about a file the far end never opened.
///
/// It has no engine coupling to give up: every name it needs — [`TreeNode`],
/// [`NodeLevel::from_str_label`], [`level_from_node_id`], [`derive_parent_id`]
/// and [`estimate_tokens`] — is **contract** vocabulary, defined in
/// `tinymemory-bus` and reached here through
/// [`crate::openhuman::memory::api::tree`]. The engine crate re-exported the
/// same items, so the types this produces are the same types it produced
/// before, not host look-alikes.
///
/// # Two faithfulness notes for a reviewer
///
/// - **The engine's signature was `Result<TreeNode>`; this one is not.** Its
///   body contains no `?` and no `Err` construction — every field falls back
///   to a derived or default value, which is the documented point of it ("does
///   not fail on malformed frontmatter", which is also what lets a truncated
///   write go undetected). So the `.ok()?` the call site used could never skip
///   a namespace, and dropping the `Result` changes no behaviour. Restore the
///   `Result` if this ever grows a genuine failure.
/// - **Timestamps fall back to `UNIX_EPOCH`, not to `now()`**, and
///   `updated_at` falls back to `created_at` rather than to the epoch
///   independently. `collect_profile_tree_root_summaries` returns
///   `node.updated_at` straight to the prompt builder, so a node with no
///   frontmatter reports 1970 — deliberately, since a missing timestamp
///   sorting as "just updated" would be worse.
fn parse_node_markdown(
    raw: &str,
    namespace: &str,
    node_id: &str,
) -> crate::openhuman::memory::api::tree::TreeNode {
    use crate::openhuman::memory::api::tree::{
        derive_parent_id, estimate_tokens, level_from_node_id, NodeLevel, TreeNode,
    };
    use chrono::{DateTime, Utc};

    let (frontmatter, body_raw) = split_frontmatter(raw);
    let body = body_raw.trim_end().to_string();

    let level = frontmatter
        .get("level")
        .and_then(|v| NodeLevel::from_str_label(v))
        .unwrap_or_else(|| level_from_node_id(node_id));
    let parent_id = frontmatter
        .get("parent_id")
        .and_then(|v| {
            let trimmed = v.trim().trim_matches('"');
            if trimmed == "~" || trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .or_else(|| derive_parent_id(node_id));
    let token_count = frontmatter
        .get("token_count")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or_else(|| estimate_tokens(&body));
    let child_count = frontmatter
        .get("child_count")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let created_at = frontmatter
        .get("created_at")
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let updated_at = frontmatter
        .get("updated_at")
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(created_at);
    let metadata = frontmatter.get("metadata").map(|v| v.to_string());

    TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level,
        parent_id,
        summary: body,
        token_count,
        child_count,
        created_at,
        updated_at,
        metadata,
    }
}

/// Split markdown into a (frontmatter key-value map, body text) pair.
///
/// Ported alongside [`parse_node_markdown`] — it is that function's only
/// helper and is `pub(crate)` in the engine for the engine's own callers, none
/// of which exist here.
///
/// Looks for a leading `---` fence and the first subsequent `\n---`; each
/// `key: value` line in between is parsed with a single `find(':')` split
/// (values are trimmed and unwrapped of one layer of surrounding `"`). If the
/// content doesn't start with `---`, or no closing fence is found, the whole
/// input is returned unmodified as the body with an empty map — this function
/// never errors, it degrades to "no frontmatter" instead. That degradation is
/// what makes [`parse_node_markdown`] total.
fn split_frontmatter(raw: &str) -> (std::collections::HashMap<String, String>, String) {
    let mut map = std::collections::HashMap::new();
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (map, raw.to_string());
    }
    let after_open = &trimmed[3..];
    if let Some(close_pos) = after_open.find("\n---") {
        let fm_block = &after_open[..close_pos];
        let body = after_open[close_pos + 4..]
            .trim_start_matches('\n')
            .to_string();
        for line in fm_block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_string();
                let raw_value = line[colon_pos + 1..].trim();
                let value = serde_json::from_str::<String>(raw_value)
                    .unwrap_or_else(|_| raw_value.trim_matches('"').to_string());
                map.insert(key, value);
            }
        }
        (map, body)
    } else {
        (map, raw.to_string())
    }
}

/// Sanitize a learned memory entry before injecting into the system prompt.
/// Strips raw data, limits length, and removes potential secrets.
pub(super) fn sanitize_learned_entry(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Truncate to a safe length
    let max_len = 200;
    let sanitized: String = trimmed.chars().take(max_len).collect();
    // Strip anything that looks like a secret/token
    if sanitized.contains("Bearer ")
        || sanitized.contains("sk-")
        || sanitized.contains("ghp_")
        || sanitized.contains("-----BEGIN")
    {
        return "[redacted: potential secret]".to_string();
    }
    sanitized
}

#[cfg(test)]
pub(crate) use super::transcript;
#[cfg(test)]
pub(crate) use super::turn_checkpoint::assistant_message_has_tool_calls;
#[cfg(test)]
pub(crate) use super::types::Agent;
#[cfg(test)]
pub(crate) use crate::openhuman::agent::context::prompt::LearnedContextData;
#[cfg(test)]
pub(crate) use anyhow::Result;

#[cfg(test)]
#[path = "../turn_tests.rs"]
mod tests;
