
/// Reads one builder tool result's content as a failure description, or
/// `None` when it reads as success/progress (a `workflow_proposal` payload,
/// or an `"ok": true` report). The whole body is the description, never one
/// hardcoded field, so this stays correct regardless of which fields a given
/// tool uses to explain its failure.
fn describe_tool_result_blocker(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.get("type").and_then(Value::as_str) == Some("workflow_proposal") {
            return None; // Success: a proposal was emitted.
        }
        if let Some(ok) = value.get("ok").and_then(Value::as_bool) {
            return if ok { None } else { Some(value.to_string()) };
        }
        // Some other structured payload with no `ok`/`type` marker this
        // function recognises — not confidently a blocker, skip it.
        return None;
    }
    // Non-JSON content: a hard-gate rejection (`ToolResult::error`) puts the
    // plain error message straight into the content — since every builder
    // tool's SUCCESS shape is JSON (a proposal or a `{ ok, ... }` report), a
    // bare string here is, by elimination, an error message.
    Some(trimmed.to_string())
}

/// Scans an agent run's conversation history for the workflow proposal a builder
/// tool emitted. `propose_workflow` / `revise_workflow` / `save_workflow` all
/// return a self-describing `{ "type": "workflow_proposal", … }` JSON string as
/// their tool result, so we match on that (the same gate the frontend uses) and
/// return the LAST one — the most recent proposal in the turn.
fn extract_workflow_proposal(
    history: &[crate::openhuman::agent::messages::ConversationMessage],
) -> Option<Value> {
    use crate::openhuman::agent::messages::ConversationMessage;
    let mut latest = None;
    for message in history {
        if let ConversationMessage::ToolResults(results) = message {
            for result in results {
                if let Ok(value) = serde_json::from_str::<Value>(&result.content) {
                    if value.get("type").and_then(Value::as_str) == Some("workflow_proposal") {
                        latest = Some(value);
                    }
                }
            }
        }
    }
    latest
}

/// Lists persisted workflow suggestions. `status` filters to one lifecycle
/// state (the UI passes `New` for the active "Suggested for you" cards); `None`
/// returns every status.
pub async fn flows_list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
) -> Result<RpcOutcome<Vec<FlowSuggestion>>, String> {
    let suggestions = store::list_suggestions(config, status, 100).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(suggestions, "suggestions listed"))
}

/// Marks a suggestion `dismissed` (the user rejected the card). The row is kept
/// so a later discovery run dedupes against it and won't re-surface the idea.
pub async fn flows_dismiss_suggestion(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let found = store::set_suggestion_status(config, id, SuggestionStatus::Dismissed)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "dismissed": found }),
        "suggestion dismissed",
    ))
}

/// Marks a suggestion `built` — called by the frontend after the user saves a
/// flow authored from this suggestion, so it drops out of the active cards.
pub async fn flows_mark_suggestion_built(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let found = store::set_suggestion_status(config, id, SuggestionStatus::Built)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "built": found }),
        "suggestion marked built",
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Connector onboarding (Phase 5, item 18) — which toolkits a graph needs
// ─────────────────────────────────────────────────────────────────────────────

/// The set of Composio toolkits currently connected (lowercased), derived from
/// the same picker source the node-config credential dropdown uses.
pub(crate) async fn connected_toolkits(config: &Config) -> std::collections::HashSet<String> {
    match flows_list_connections(config).await {
        Ok(outcome) => outcome
            .value
            .iter()
            .filter_map(|c| c.toolkit.as_deref())
            .map(|t| t.to_ascii_lowercase())
            .collect(),
        Err(e) => {
            tracing::warn!(target: "flows", error = %e, "[flows] connected_toolkits: could not list connections — treating all as unconnected");
            std::collections::HashSet::new()
        }
    }
}

/// The Composio toolkits a graph needs (from its `tool_call` slugs and any
/// `app_event` trigger), each tagged connected/missing — the data behind the
/// canvas/proposal "Connect <toolkit>" CTAs (audit Phase 5, item 18). Native
/// `oh:` tools and `http_request` nodes need no Composio connection and are
/// skipped.
pub async fn compute_required_connections(config: &Config, graph: &WorkflowGraph) -> Vec<Value> {
    use tinymemory_api::composio::toolkit_from_slug;

    // Collect required toolkits (deduped, order-preserving).
    let mut required: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |tk: String| {
        let tk = tk.to_ascii_lowercase();
        if !tk.is_empty() && seen.insert(tk.clone()) {
            required.push(tk);
        }
    };

    for node in &graph.nodes {
        if node.kind == NodeKind::ToolCall {
            if let Some(slug) = node.config.get("slug").and_then(Value::as_str) {
                // Native OpenHuman tools (`oh:<name>`) need no connection.
                if slug.starts_with("oh:") {
                    continue;
                }
                if let Some(tk) = toolkit_from_slug(slug) {
                    push(tk.to_string());
                }
            }
        }
    }
    // An app_event trigger names its toolkit directly.
    if let Some(trigger) = graph.trigger() {
        if let Some(tk) = trigger.config.get("toolkit").and_then(Value::as_str) {
            push(tk.to_string());
        }
    }

    if required.is_empty() {
        return Vec::new();
    }

    let connected = connected_toolkits(config).await;
    required
        .into_iter()
        .map(|toolkit| {
            let status = if connected.contains(&toolkit) {
                "connected"
            } else {
                "missing"
            };
            json!({ "toolkit": toolkit, "status": status })
        })
        .collect()
}

/// RPC: compute the toolkits a candidate graph needs and their connected
/// status, so the canvas/proposal can render "Connect <toolkit>" CTAs.
pub async fn flows_required_connections(
    config: &Config,
    graph_json: Value,
) -> Result<RpcOutcome<Value>, String> {
    let graph = migrate_and_deserialize_graph(graph_json)?;
    let required = compute_required_connections(config, &graph).await;
    Ok(RpcOutcome::single_log(
        json!({ "required_connections": required }),
        "required connections computed",
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Save-time approval manifest (consolidated pre-authorization card)
// ─────────────────────────────────────────────────────────────────────────────

/// Statically compute the "approval manifest" for a graph: every ApprovalGate
/// permission a run of this flow will prompt for, so the save+enable card can
/// ask for all of them in one shot instead of parking the run node-by-node.
///
/// Mirrors — never re-implements — the runtime gating in
/// `crate::openhuman::flows::tinyflows::caps` (`OpenHumanTools::invoke` /
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
    use crate::openhuman::flows::tinyflows::caps::classify_composio_action_for_tier;
    use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};

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
                    Some(s)
                        if s.starts_with(
                            crate::openhuman::flows::tinyflows::caps::NATIVE_TOOL_PREFIX,
                        ) =>
                    {
                        let tool_name = s
                            .trim_start_matches(
                                crate::openhuman::flows::tinyflows::caps::NATIVE_TOOL_PREFIX,
                            )
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
                        let class = crate::openhuman::runtime::node::ops::classify_tool_call(
                            config, &tool_name, &args,
                        )
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
fn split_manifest_trust(
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
) -> Result<RpcOutcome<Value>, String> {
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

    let gate = crate::openhuman::security::approval::ApprovalGate::try_global();
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
    Ok(RpcOutcome::single_log(
        json!({
            "entries": entries,
            "missing": missing,
            "already_trusted": already_trusted,
            "gate_installed": gate_installed,
        }),
        log,
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Catalog RPCs for the UI (Phase 5, item 16) — one implementation, two consumers
// ─────────────────────────────────────────────────────────────────────────────

/// Searches the live Composio tool catalog (secret-free) — the RPC the in-canvas
/// tool browser calls, reusing the exact same core as the agent's
/// `search_tool_catalog` tool so the two can't drift.
pub async fn flows_search_tool_catalog(
    config: &Config,
    query: &str,
    toolkit: Option<&str>,
    limit: usize,
) -> Result<RpcOutcome<Value>, String> {
    tracing::debug!(target: "flows", %query, toolkit = toolkit.unwrap_or("<all>"), "[flows] flows_search_tool_catalog: searching live catalog");
    let tools =
        crate::openhuman::flows::builder_tools::search_live_catalog(config, query, toolkit, limit)
            .await;
    Ok(RpcOutcome::single_log(
        json!({ "tools": tools }),
        "tool catalog searched",
    ))
}

/// Resolves the toolkit for a *single action* slug, rejecting anything that is
/// not shaped `<TOOLKIT>_<ACTION>`.
///
/// [`tinymemory_api::composio::toolkit_from_slug`] falls back to the whole
/// string when there is no `_`, so it answers `Some` for every non-empty input
/// — `toolkit_from_slug("nodashhere") == Some("nodashhere")`. That permissive
/// fall-back is load-bearing for `compute_required_connections`, which maps
/// toolkit-ish tokens and legitimately wants the whole string back, so the
/// stricter rule belongs to this caller rather than to the shared helper.
///
/// A contract fetch returns *one action*, so a slug with no action segment
/// cannot name anything it could return; rejecting it here also spares the
/// caller a pointless catalog round trip (which takes a per-toolkit fetch lock)
/// before failing with an unrelated "could not fetch the catalog" message.
///
/// The multi-segment toolkit prefixes (`MICROSOFT_TEAMS_`, `ONE_DRIVE_`,
/// `ZOHO_MAIL_`) all end in `_`, so a real action under one of them always has
/// non-empty segments either side of its first `_` and passes unchanged.
pub(super) fn toolkit_for_contract_slug(slug: &str) -> Option<String> {
    let trimmed = slug.trim();
    let (toolkit_segment, action_segment) = trimmed.split_once('_')?;
    if toolkit_segment.is_empty() || action_segment.is_empty() {
        return None;
    }
    tinymemory_api::composio::toolkit_from_slug(trimmed)
}

/// Fetches one Composio action's full contract (secret-free) — the RPC the
/// canvas tool browser calls to fill in an action's arg schema, reusing the same
/// core as the agent's `get_tool_contract` tool.
pub async fn flows_get_tool_contract(
    config: &Config,
    slug: &str,
) -> Result<RpcOutcome<Value>, String> {
    let trimmed = slug.trim();
    // Shape-check before `toolkit_from_slug`, and before any I/O: the shared
    // helper is deliberately permissive, so an unshaped slug would otherwise
    // fall through to a catalog round trip and fail with an unrelated message.
    // The message quotes the caller's own slug, not `trimmed` — reporting the
    // trimmed form named an empty string back at whoever sent whitespace.
    let Some(toolkit) = toolkit_for_contract_slug(trimmed) else {
        return Err(format!(
            "Could not extract a toolkit from slug '{slug}' — it must look like \
             '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
        ));
    };
    tracing::debug!(target: "flows", slug = %trimmed, %toolkit, "[flows] flows_get_tool_contract: fetching contract");
    let Some(catalog) =
        crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog(config, &toolkit)
            .await
    else {
        return Err(format!(
            "Could not fetch the live Composio catalog for toolkit '{toolkit}'."
        ));
    };
    match catalog
        .iter()
        .find(|c| c.slug.eq_ignore_ascii_case(trimmed))
    {
        Some(contract) => {
            let contract =
                crate::openhuman::flows::tinyflows::caps::apply_probe_override(contract.clone());
            let value = serde_json::to_value(&contract).map_err(|e| e.to_string())?;
            Ok(RpcOutcome::single_log(
                json!({ "contract": value }),
                "tool contract fetched",
            ))
        }
        None => Err(format!(
            "'{trimmed}' is not a real action in the '{toolkit}' toolkit's live catalog."
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core-managed local drafts (F5) — the shared agent/canvas working copy
// ─────────────────────────────────────────────────────────────────────────────

/// Creates a new draft (a durable, non-live working copy) from a graph.
pub fn flows_draft_create(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: crate::openhuman::flows::DraftOrigin,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowDraft>, String> {
    let draft = draft_store::create_draft(config, flow_id, name, graph, origin)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(draft, "draft created"))
}

/// Reads a draft by id (errors if it does not exist).
pub fn flows_draft_get(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowDraft>, String> {
    let draft = draft_store::get_draft(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("draft '{id}' not found"))?;
    Ok(RpcOutcome::single_log(draft, format!("draft loaded: {id}")))
}

/// Patches a draft's `name`/`graph`/`flow_id` (any `Some` applied) and bumps
/// `updated_at`.
pub fn flows_draft_update(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowDraft>, String> {
    let draft =
        draft_store::update_draft(config, id, name, graph, flow_id).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(draft, "draft updated"))
}

/// Lists all drafts, newest-updated first.
pub fn flows_draft_list(
    config: &Config,
) -> Result<RpcOutcome<Vec<crate::openhuman::flows::FlowDraft>>, String> {
    let drafts = draft_store::list_drafts(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(drafts, "drafts listed"))
}

/// Deletes a draft by id (idempotent — reports whether a file was removed).
pub fn flows_draft_delete(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let deleted = draft_store::delete_draft(config, id).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "deleted": deleted }),
        "draft deleted",
    ))
}

/// Promotes a draft into a saved flow, then removes the draft file.
///
/// Runs the SAME create/update gates as a normal save (structural validation,
/// the forced `require_approval` floor for side-effect graphs, born-disabled
/// for automatic triggers) — a draft is never a back-door around them. A draft
/// with a `flow_id` updates that flow; otherwise it creates a new one. The
/// draft file is deleted only on a successful promote.
pub async fn flows_draft_promote(
    config: &Config,
    id: &str,
    require_approval: Option<bool>,
) -> Result<RpcOutcome<Flow>, String> {
    let draft = draft_store::get_draft(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("draft '{id}' not found"))?;

    tracing::debug!(
        target: "flows",
        draft_id = %id,
        promotes_to = draft.flow_id.as_deref().unwrap_or("<new flow>"),
        "[flows] flows_draft_promote: promoting draft through the create/update gates"
    );

    let outcome = match &draft.flow_id {
        Some(flow_id) => {
            flows_update(
                config,
                flow_id,
                Some(draft.name.clone()),
                Some(draft.graph.clone()),
                require_approval,
                None,
            )
            .await?
        }
        None => {
            flows_create(
                config,
                draft.name.clone(),
                draft.graph.clone(),
                require_approval.unwrap_or(false),
            )
            .await?
        }
    };

    // Only remove the draft once the flow write succeeded.
    if let Err(e) = draft_store::delete_draft(config, id) {
        tracing::warn!(target: "flows", draft_id = %id, error = %e, "[flows] flows_draft_promote: flow saved but draft file could not be removed");
    }
    Ok(outcome)
}
