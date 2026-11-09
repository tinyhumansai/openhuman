use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Save-time approval manifest (consolidated pre-authorization card)
// ─────────────────────────────────────────────────────────────────────────────

/// Statically compute the "approval manifest" for a graph: every ApprovalGate
/// permission a run of this flow will prompt for, so the save+enable card can
/// ask for all of them in one shot instead of parking the run node-by-node.
///
/// Mirrors — never re-implements — the runtime gating in
/// `crate::flows::caps_host` (`OpenHumanTools::invoke` /
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
pub async fn compute_approval_manifest(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Result<Vec<Value>, String> {
    compute_approval_manifest_json(ctx, config, graph.clone()).await
}

async fn compute_approval_manifest_json(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<Value>, String> {
    let registration =
        crate::flows::authoring_host::register(std::sync::Arc::new(config.clone()), ctx.clone())
            .await?;
    let proxy = crate::modules::tinyflows::catalog_proxy(config).await?;
    let entries: Vec<Value> = proxy
        .call(
            tinyflows_bus::catalog::members::APPROVAL_MANIFEST,
            &(tinyflows_bus::catalog::ApprovalManifestRequest {
                graph,
                native_prefix: crate::flows::caps_host::NATIVE_TOOL_PREFIX.into(),
                host: registration.catalog.clone(),
            },),
        )
        .await
        .map_err(|error| error.wire_message())?;
    tracing::debug!(target: "flows", entries = entries.len(), "[flows] approval manifest computed");
    Ok(entries)
}

/// Approval manifest for a portable graph payload from the Copilot bus.
pub(crate) async fn approval_manifest_json(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<Value>, String> {
    compute_approval_manifest_json(ctx, config, graph).await
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
    ctx: &crate::core::runtime::DispatchCtx,
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
            (
                serde_json::to_value(flow.graph).map_err(|e| e.to_string())?,
                Some(id.to_string()),
            )
        }
        (None, Some(graph_json)) => (graph_json, None),
        (None, None) => return Err("provide 'id' or 'graph'".to_string()),
    };

    let entries = compute_approval_manifest_json(ctx, config, graph).await?;

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

    let (mut missing, mut already_trusted) = (Vec::new(), Vec::new());
    if gate_installed {
        for entry in &entries {
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
    }

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
