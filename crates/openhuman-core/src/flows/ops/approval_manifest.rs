use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Save-time approval manifest (consolidated pre-authorization card)
// ─────────────────────────────────────────────────────────────────────────────

/// Statically compute the "approval manifest" for a graph: every ApprovalGate
/// permission a run of this flow will prompt for, so the save+enable card can
/// ask for all of them in one shot instead of parking the run node-by-node.
///
/// Mirrors — never re-implements — the runtime gating in
/// `crate::flows::tinyflows::caps` (`OpenHumanTools::invoke` /
/// `OpenHumanHttp` / `OpenHumanCode`) and `approval::gate`'s Workflow-origin
/// branch. Because Rule 2 (`enforce_side_effect_approval`) forces
/// `require_approval: true` onto every graph with outbound side-effect nodes,
/// a run parks on EVERY gated node that lacks `(flow_id, tool_name)` trust —
/// so the manifest is precisely "the trust keys a fully pre-authorized run
/// needs".
///
/// Entry `kind`s:
/// - `"approvable"` — will park; pre-approving `tool_name` clears it.
/// - `"blocked"` — the autonomy tier `Block`s the node's class outright
///   (`enforce_node_tier_gate` refuses before dispatch); NOT approvable from
///   the card — shown informationally so the user learns at save time, not
///   at run time.
/// - `"dynamic"` — the node's slug is an inline `=` expression resolved from
///   runtime data; its trust key is unknowable at save time and it stays
///   gated (best-effort disclosure).
/// - `"agent"` — an `agent` node with an `agent_ref` runs a full harness turn
///   whose inner tool calls cannot be enumerated statically; disclosed so the
///   card never over-promises "zero prompts".
///
/// Curated Composio Read actions are excluded entirely: `CommandClass::Read`
/// is `Allow` under every tier and the runtime skips the gate for them, so
/// listing them would request grants that are never checked.
pub async fn compute_approval_manifest(config: &Config, graph: &WorkflowGraph) -> Vec<Value> {
    use crate::flows::tinyflows::caps::classify_composio_action_for_tier;
    use crate::security::{CommandClass, GateDecision, SecurityPolicy};

    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);

    let mut entries: Vec<Value> = Vec::new();
    // Approvable/blocked rows dedupe on the trust key (`tool_name`) — two
    // nodes calling the same tool need one grant, so they get one row.
    let mut seen_tools: HashSet<String> = HashSet::new();

    let push_gated = |entries: &mut Vec<Value>,
                      seen_tools: &mut HashSet<String>,
                      node_id: &str,
                      tool_name: String,
                      label: String,
                      class: CommandClass| {
        if !seen_tools.insert(tool_name.clone()) {
            return;
        }
        let kind = if security.gate_decision(class) == GateDecision::Block {
            "blocked"
        } else {
            "approvable"
        };
        entries.push(json!({
            "kind": kind,
            "node_id": node_id,
            "tool_name": tool_name,
            "label": label,
            "class": format!("{class:?}"),
        }));
    };

    for node in &graph.nodes {
        match node.kind {
            NodeKind::HttpRequest => {
                let url = node
                    .config
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("HTTP request");
                push_gated(
                    &mut entries,
                    &mut seen_tools,
                    &node.id,
                    "flows_http_request".to_string(),
                    format!("Call {url}"),
                    CommandClass::Network,
                );
            }
            NodeKind::Code => {
                push_gated(
                    &mut entries,
                    &mut seen_tools,
                    &node.id,
                    "flows_code".to_string(),
                    "Run sandboxed code".to_string(),
                    CommandClass::Write,
                );
            }
            NodeKind::ToolCall => {
                let slug = node.config.get("slug").and_then(Value::as_str);
                match slug {
                    Some(s) if s.trim_start().starts_with('=') => {
                        tracing::debug!(
                            target: "flows",
                            node_id = %node.id,
                            "[flows] approval manifest: dynamic `=` slug — cannot pre-approve"
                        );
                        entries.push(json!({
                            "kind": "dynamic",
                            "node_id": node.id,
                            "label": "Tool chosen at run time",
                        }));
                    }
                    Some(s) if s.starts_with(crate::flows::tinyflows::caps::NATIVE_TOOL_PREFIX) => {
                        let tool_name = s
                            .trim_start_matches(crate::flows::tinyflows::caps::NATIVE_TOOL_PREFIX)
                            .trim()
                            .to_string();
                        if tool_name.is_empty() {
                            continue; // structurally invalid; validate rejects elsewhere
                        }
                        let args = node.config.get("args").cloned().unwrap_or(json!({}));
                        // Same classifier the runtime dispatch uses. Args may
                        // contain unresolved `=` bindings, so a classification
                        // error (unknown tool, etc.) degrades conservatively
                        // to Network — over-asking is safe, under-asking
                        // re-introduces the mid-run park this feature removes.
                        let class =
                            crate::tools::native_ops::classify_tool_call(config, &tool_name, &args)
                                .unwrap_or(CommandClass::Network);
                        push_gated(
                            &mut entries,
                            &mut seen_tools,
                            &node.id,
                            tool_name.clone(),
                            format!("Use tool {tool_name}"),
                            class,
                        );
                    }
                    Some(s) if !s.trim().is_empty() => {
                        let class = classify_composio_action_for_tier(s).await;
                        if class == CommandClass::Read {
                            // Curated read: runtime never gates it.
                            continue;
                        }
                        push_gated(
                            &mut entries,
                            &mut seen_tools,
                            &node.id,
                            s.to_string(),
                            format!("Use {s}"),
                            class,
                        );
                    }
                    _ => {}
                }
            }
            NodeKind::Agent
                if node
                    .config
                    .get("agent_ref")
                    .and_then(Value::as_str)
                    .is_some_and(|r| !r.trim().is_empty()) =>
            {
                entries.push(json!({
                    "kind": "agent",
                    "node_id": node.id,
                    "label": "AI step — may ask for permission for its own actions",
                }));
            }
            _ => {}
        }
    }

    tracing::debug!(
        target: "flows",
        entries = entries.len(),
        "[flows] approval manifest computed"
    );
    entries
}

/// Splits a manifest's approvable entries into the trust keys a run will have to
/// ask for and the ones the flow already holds a grant for.
///
/// With no gate installed both lists are empty: nothing ever parks, so nothing
/// is `missing` — and nothing was ever granted, so nothing is `already_trusted`
/// either. Naming the approvable keys there would tell the save+enable card that
/// permissions are authorized on a host that holds no grant for them.
/// `gate_installed: false` is the caller's single signal, which is exactly what
/// the sibling `approval_preauthorize_flow` reports for the same condition.
pub(super) fn split_manifest_trust(
    entries: &[Value],
    gate_installed: bool,
    trusted: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut missing: Vec<String> = Vec::new();
    let mut already_trusted: Vec<String> = Vec::new();
    if !gate_installed {
        return (missing, already_trusted);
    }
    for entry in entries {
        if entry.get("kind").and_then(Value::as_str) != Some("approvable") {
            continue;
        }
        let Some(tool_name) = entry.get("tool_name").and_then(Value::as_str) else {
            continue;
        };
        if trusted.contains(tool_name) {
            already_trusted.push(tool_name.to_string());
        } else {
            missing.push(tool_name.to_string());
        }
    }
    (missing, already_trusted)
}

/// RPC: the approval manifest for a saved flow (by `id`) or a candidate
/// `graph`, joined against the flow's existing `flow_tool_trust` grants so
/// the save+enable card can ask only for what's missing.
///
/// With the approval gate uninstalled (`OPENHUMAN_APPROVAL_GATE=0`) nothing
/// ever parks and nothing was ever granted, so `missing` and `already_trusted`
/// are both empty by definition and the card never shows — `gate_installed:
/// false` is the only signal in that case.
pub async fn flows_approval_manifest(
    config: &Config,
    id: Option<&str>,
    graph_json: Option<Value>,
) -> Result<Outcome<Value>, String> {
    tracing::debug!(target: "flows", id = ?id, has_graph = graph_json.is_some(), "[flows] flows_approval_manifest: entry");
    let (graph, flow_id) = match (id, graph_json) {
        (Some(id), _) => {
            let flow = store::get_flow(config, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("flow not found: {id}"))?;
            // `store::get_flow` already returns a migrated, deserialized graph.
            (flow.graph, Some(id.to_string()))
        }
        (None, Some(graph_json)) => (migrate_and_deserialize_graph(graph_json)?, None),
        (None, None) => return Err("provide 'id' or 'graph'".to_string()),
    };

    let entries = compute_approval_manifest(config, &graph).await;

    let gate = crate::security::approval::ApprovalGate::try_global();
    let gate_installed = gate.is_some();
    let trusted: HashSet<String> = match (&gate, &flow_id) {
        (Some(gate), Some(flow_id)) => gate
            .list_flow_trust(flow_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect(),
        _ => HashSet::new(),
    };

    let (missing, already_trusted) = split_manifest_trust(&entries, gate_installed, &trusted);

    let log = format!(
        "[flows] approval manifest: {} entr{}, {} missing grant(s)",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
        missing.len()
    );
    tracing::debug!(target: "flows", entries = entries.len(), missing = missing.len(), gate_installed, "[flows] flows_approval_manifest: exit");
    Ok(Outcome::single_log(
        json!({
            "entries": entries,
            "missing": missing,
            "already_trusted": already_trusted,
            "gate_installed": gate_installed,
        }),
        log,
    ))
}
