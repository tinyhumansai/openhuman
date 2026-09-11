
/// Hard gate: for every Composio `tool_call` node carrying a `connection_ref`,
/// prove the ref names a real connected account of the SAME toolkit as the
/// slug. Fetches the live connection list once (same source
/// [`flows_list_connections`] reads) and delegates the pure matching to
/// [`validate_connection_refs_against`].
///
/// Fail-open on I/O: if the Composio connection list is unreachable (backend
/// outage), the id-existence check is SKIPPED (a `tracing::debug!` records it)
/// so a real connection is never false-rejected during an outage — but the
/// toolkit-mismatch check, which needs no I/O, still runs.
pub(crate) async fn validate_connection_refs(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let connections: Option<Vec<FlowConnection>> =
        match crate::openhuman::integrations::composio::ops::composio_list_connections(config).await
        {
            Ok(outcome) => Some(build_flow_connections(
                outcome.value.connections,
                Vec::new(),
                // Identity isn't needed for this existence/toolkit-mismatch
                // check — only `connection_ref` and `toolkit` are read.
                &[],
            )),
            Err(e) => {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] connection-ref check: composio connection list unavailable — \
                     skipping id-existence check (fail-open); toolkit-mismatch check still runs"
                );
                None
            }
        };
    validate_connection_refs_against(graph, connections.as_deref())
}

/// Pure connection-ref validator (no I/O) so the gate's decision logic is
/// unit-testable without a live Composio backend. `connections` is `Some(list)`
/// when the live connection list was fetched (possibly empty — a genuine "no
/// connections" state), or `None` when it was unavailable (fail-open: the
/// id-existence check is skipped, only the toolkit-mismatch check runs).
fn validate_connection_refs_against(
    graph: &WorkflowGraph,
    connections: Option<&[FlowConnection]>,
) -> Vec<String> {
    use tinymemory_api::composio::toolkit_from_slug;

    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs resolve at runtime; native `oh:` tools have no
        // Composio connection to name.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        // A MISSING `connection_ref` stays allowed (unchanged): a Composio
        // tool_call with no ref runs against the ambient signed-in account and
        // the flow prompts for a connection at first run.
        let Some(conn_ref) = node.config.get("connection_ref").and_then(Value::as_str) else {
            continue;
        };
        if conn_ref.trim().is_empty() {
            continue;
        }
        let Some(slug_toolkit) = toolkit_from_slug(slug) else {
            continue;
        };

        let Some((ref_toolkit, ref_id)) = parse_composio_connection_ref(conn_ref) else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %conn_ref,
                matched = false,
                "[flows] connection-ref check: malformed ref — rejecting"
            );
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` is malformed — a Composio account ref \
                 must look like `composio:<toolkit>:<connection_id>` (e.g. \
                 `composio:{slug_toolkit}:<id>`). Call list_flow_connections and copy a \
                 `connection_ref` value verbatim.",
                node.id
            ));
            continue;
        };

        // Toolkit segment vs the slug's toolkit — needs no I/O.
        if !ref_toolkit.eq_ignore_ascii_case(&slug_toolkit) {
            let suggestion = connections
                .and_then(|conns| first_connection_ref_for_toolkit(conns, &slug_toolkit));
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_toolkit,
                %ref_id,
                matched = false,
                "[flows] connection-ref check: toolkit segment does not match the slug's toolkit — rejecting"
            );
            let hint = match suggestion {
                Some(r) => format!(" — did you mean `{r}`?"),
                None => format!(
                    " — no `{slug_toolkit}` account is connected; connect one with \
                     composio_connect (or ask the user to), then use its `connection_ref`"
                ),
            };
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` names the `{ref_toolkit}` toolkit but the \
                 tool_call slug `{slug}` is a `{slug_toolkit}` action{hint}.",
                node.id
            ));
            continue;
        }

        // Existence check: the id must name a real connected account of this
        // toolkit. Skipped (fail-open) when the connection list is unavailable.
        let Some(conns) = connections else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_id,
                "[flows] connection-ref check: toolkit matches; id-existence check skipped (connections unavailable)"
            );
            continue;
        };
        // The id must belong to a connection OF THIS TOOLKIT — not merely
        // exist somewhere. The transcript bug was a real TIKTOK connection id
        // stamped onto a `composio:twitter:` ref: the id exists globally, but
        // it is not a Twitter account, so it must still be rejected.
        let id_exists = conns.iter().any(|c| {
            c.toolkit
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&slug_toolkit))
                && parse_composio_connection_ref(&c.connection_ref)
                    .is_some_and(|(_, cid)| cid.eq_ignore_ascii_case(ref_id))
        });
        if id_exists {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_id,
                matched = true,
                "[flows] connection-ref check: ref resolves to a real connected account — ok"
            );
            continue;
        }
        // Unknown id. Name the right ref for this toolkit if one exists.
        match first_connection_ref_for_toolkit(conns, &slug_toolkit) {
            Some(r) => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    toolkit = %slug_toolkit,
                    %ref_id,
                    matched = false,
                    "[flows] connection-ref check: unknown id; toolkit has a different connected account — rejecting"
                );
                errors.push(format!(
                    "Node '{}': `connection_ref` `{conn_ref}` does not match any connected \
                     `{slug_toolkit}` account — did you mean `{r}`? Call list_flow_connections and \
                     copy a `connection_ref` value verbatim.",
                    node.id
                ));
            }
            None => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    toolkit = %slug_toolkit,
                    %ref_id,
                    matched = false,
                    "[flows] connection-ref check: no connected account for this toolkit — rejecting"
                );
                errors.push(format!(
                    "Node '{}': `connection_ref` `{conn_ref}` names a `{slug_toolkit}` account, but \
                     no `{slug_toolkit}` account is connected — connect one with composio_connect \
                     (or ask the user to), then use its `connection_ref`.",
                    node.id
                ));
            }
        }
    }
    errors
}

/// Validates a candidate graph without persisting it — the same
/// migrate/validate path `flows_create` and `ProposeWorkflowTool` use — and
/// reports structural errors alongside non-fatal trigger warnings
/// ([`graph_trigger_warnings`]). Backs `openhuman.flows_validate` (PHASE 3c):
/// an authoring surface can call this to preview validity + warnings before a
/// save. Pure (no persistence, no config) — `valid == false` is a normal
/// result, NOT an `Err`; `Err` is reserved for internal serialization faults
/// (there are none on this path today).
pub fn flows_validate(graph_json: Value) -> RpcOutcome<crate::openhuman::flows::FlowValidation> {
    use crate::openhuman::flows::FlowValidation;
    tracing::debug!(target: "flows", "[flows] flows_validate: validating candidate graph");
    // Split migrate/deserialize (a genuinely single failure) from structural
    // validation (which can surface many problems at once). A pre-validation
    // failure short-circuits with one error; a deserializable graph is then run
    // through `validate_all` so the author sees every structural problem in one
    // pass instead of one round-trip per error.
    let graph = match migrate_and_deserialize_graph(graph_json) {
        Ok(graph) => graph,
        Err(error) => {
            tracing::debug!(target: "flows", %error, "[flows] flows_validate: graph could not be migrated/parsed");
            return RpcOutcome::single_log(
                FlowValidation {
                    valid: false,
                    errors: vec![error.clone()],
                    error_details: vec![crate::openhuman::flows::FlowValidationError {
                        code: "unparseable_graph".to_string(),
                        message: error,
                        node_id: None,
                        field: None,
                    }],
                    warnings: Vec::new(),
                },
                "flow validation failed",
            );
        }
    };

    let structural = tinyflows::validate::validate_all(&graph);
    if !structural.is_empty() {
        let error_details: Vec<_> = structural.iter().map(to_flow_validation_error).collect();
        let errors: Vec<String> = error_details.iter().map(|e| e.message.clone()).collect();
        tracing::debug!(
            target: "flows",
            error_count = errors.len(),
            "[flows] flows_validate: graph is structurally invalid"
        );
        return RpcOutcome::single_log(
            FlowValidation {
                valid: false,
                errors,
                error_details,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }

    let error_details = engine_compatibility_errors(&graph);
    if !error_details.is_empty() {
        let errors = error_details
            .iter()
            .map(|error| error.message.clone())
            .collect();
        tracing::debug!(
            target: "flows",
            error_count = error_details.len(),
            "[flows] flows_validate: graph uses an unsupported engine topology"
        );
        return RpcOutcome::single_log(
            FlowValidation {
                valid: false,
                errors,
                error_details,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }

    let warnings = graph_trigger_warnings(&graph);
    for warning in &warnings {
        tracing::warn!(target: "flows", warning = %warning, "[flows] flows_validate: non-fatal validation warning");
    }
    tracing::debug!(
        target: "flows",
        node_count = graph.nodes.len(),
        warning_count = warnings.len(),
        "[flows] flows_validate: graph is structurally valid"
    );
    RpcOutcome::single_log(
        FlowValidation {
            valid: true,
            errors: Vec::new(),
            error_details: Vec::new(),
            warnings,
        },
        "flow validated",
    )
}

/// Imports a workflow definition WITHOUT persisting it (PHASE 4d), normalizing
/// it into a migrated + validated [`WorkflowGraph`] the UI opens as an editable
/// canvas *draft*. Two source formats, selected by `format`:
///
/// - `"native"` — a tinyflows `WorkflowGraph` JSON (the same shape
///   `flows_create` accepts). Run straight through [`validate_and_migrate_graph`].
/// - `"n8n"` — an n8n workflow export, mapped best-effort by
///   [`crate::openhuman::flows::n8n_import`] into a `WorkflowGraph` (unmapped
///   node types become annotated placeholders, expressions translated where
///   trivial) and THEN run through the same migrate + validate path, so the
///   host engine is the authority on the result's validity.
/// - `None`/`"auto"` — auto-detect: n8n exports carry a `connections` object /
///   `type`-discriminated nodes ([`n8n_import::looks_like_n8n`]); everything
///   else is treated as native.
///
/// Returns `Err` when the (post-mapping) graph is structurally invalid or the
/// JSON is unparseable — import declines rather than handing the canvas a graph
/// that can't be saved. On success the `warnings` carry every non-fatal import
/// approximation (n8n only; native import is warning-free).
///
/// Like `flows_validate`, this is pure: NO persistence, NO enablement. The
/// user's later Save (the existing `flows_create` gate) is the only write.
pub fn flows_import(
    graph_json: Value,
    format: Option<String>,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowImport>, String> {
    use crate::openhuman::flows::{n8n_import, FlowImport};

    let requested = format
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase();
    let is_n8n = match requested.as_str() {
        "n8n" => true,
        "native" | "tinyflows" => false,
        "auto" | "" => n8n_import::looks_like_n8n(&graph_json),
        other => {
            return Err(format!(
                "unknown import format '{other}' (expected 'native' or 'n8n')"
            ))
        }
    };
    tracing::debug!(
        target: "flows",
        requested_format = %requested,
        resolved = if is_n8n { "n8n" } else { "native" },
        "[flows] flows_import: importing workflow definition"
    );

    let (candidate, mut warnings) = if is_n8n {
        let mapped = n8n_import::map_n8n_workflow(&graph_json)?;
        // Re-serialize the mapped graph so it re-enters the exact same
        // migrate + validate path a native import takes (single source of truth
        // for validity), rather than trusting the mapper's in-memory graph.
        let value = serde_json::to_value(&mapped.graph).map_err(|e| e.to_string())?;
        (value, mapped.warnings)
    } else {
        (graph_json, Vec::new())
    };

    let graph = validate_and_migrate_graph(candidate)?;
    // Host-side trigger warnings apply to both formats (e.g. an imported
    // webhook trigger that this host does not yet self-fire).
    warnings.extend(graph_trigger_warnings(&graph));
    tracing::debug!(
        target: "flows",
        node_count = graph.nodes.len(),
        warning_count = warnings.len(),
        "[flows] flows_import: import normalized and validated"
    );
    Ok(RpcOutcome::single_log(
        FlowImport { graph, warnings },
        "flow imported",
    ))
}

/// Creates a new flow from a name and a raw graph JSON value.
///
/// Issue B29 (save/enable safety) — two server-side rules apply here,
/// authoritative regardless of what the caller passed, so no creation path
/// (prompt bar, scratch/template modal, proposal "save & enable", copilot
/// `save_workflow`, …) can silently hand the user an armed, unattended
/// automation:
///
/// - **Rule 1** ([`trigger_is_automatic`]): a graph whose trigger fires
///   without a human in the loop (`schedule` / `app_event` / `webhook`)
///   persists **disabled**. The user arms it explicitly via
///   `flows_set_enabled` — the same toggle already used everywhere else. A
///   `manual` trigger (or no trigger-kind discriminator at all) still
///   persists enabled: it only ever runs via an explicit `flows_run`, so
///   there is no surprise, and gating it would just add friction.
///
///   This means a caller that represents an explicit user-arming action
///   (e.g. `WorkflowProposalCard`'s "Save & enable" click,
///   `app/src/components/chat/WorkflowProposalCard.tsx`) must check the
///   returned [`Flow`]'s `enabled` field and follow up with
///   `flows_set_enabled(id, true)` when it comes back `false` — otherwise
///   the button's own label lies to the user. That follow-up call is a
///   legitimate, explicit enable, not the silent copilot auto-arm this rule
///   exists to prevent (the copilot's `save_workflow` path has no such
///   follow-up and stays disabled).
/// - **Rule 2** ([`graph_has_outbound_side_effect`]): a graph containing any
///   `tool_call` / `http_request` / `code` node — the three kinds that can
///   produce a real outbound effect — forces `require_approval: true`,
///   overriding whatever the caller passed. A read-only graph (only
///   `trigger` / `agent` / `transform` / `condition` / data-flow nodes) is
///   unaffected.
///
/// An enabled flow still has its automatic-dispatch side effect bound
/// immediately (e.g. the schedule-trigger cron job registered), reusing the
/// same [`bind_trigger`] helper `flows_set_enabled` uses — but per Rule 1
/// that now only happens for a `manual`-triggered (or trigger-kind-less)
/// flow. Best-effort, same as `flows_set_enabled`: a binding failure is
/// logged, not fatal to create.
pub async fn flows_create(
    config: &Config,
    name: String,
    graph_json: Value,
    require_approval: bool,
) -> Result<RpcOutcome<Flow>, String> {
    let graph = validate_and_migrate_graph(graph_json)?;
    ensure_config_aware_engine_compatible(config, &graph)?;

    // Rule 1: automatic triggers create DISABLED — the user must arm them
    // explicitly.
    let enabled = !trigger_is_automatic(&graph);

    // Rule 2: any outbound side-effect node forces require_approval, no
    // matter what the caller asked for.
    let (effective_require_approval, side_effect_forced) =
        enforce_side_effect_approval(&graph, require_approval);
    if side_effect_forced {
        tracing::info!(
            target: "flows",
            %name,
            "[flows] flows_create: forcing require_approval=true — graph contains outbound \
             side-effect node(s) (tool_call / http_request / code)"
        );
    }

    tracing::debug!(
        target: "flows",
        %name,
        node_count = graph.nodes.len(),
        enabled,
        require_approval = effective_require_approval,
        "[flows] flows_create: persisting new flow"
    );
    let flow = store::create_flow(config, name, graph, effective_require_approval, enabled)
        .map_err(|e| e.to_string())?;

    if flow.enabled {
        tracing::debug!(target: "flows", flow_id = %flow.id, "[flows] flows_create: flow is enabled — binding automatic-dispatch trigger");
        bind_trigger(config, &flow);
    }

    let mut logs = vec!["flow created".to_string()];
    if !enabled {
        let trigger_label = flow
            .graph
            .trigger()
            .and_then(|t| t.config.get("trigger_kind"))
            .and_then(Value::as_str)
            .unwrap_or("automatic");
        logs.push(format!(
            "Flow created DISABLED because it has an automatic trigger ({trigger_label}). \
             Enable it explicitly (flows_set_enabled) when you are ready for it to fire."
        ));
    }
    if side_effect_forced {
        logs.push(
            "require_approval forced to true because the graph contains outbound side-effect \
             nodes (tool_call / http_request / code)."
                .to_string(),
        );
    }

    publish_flow_changed(&flow.id, "created", "system");
    Ok(RpcOutcome::new(flow, logs))
}

/// Duplicates a saved flow: creates an independent copy of its graph under a
/// new id/timestamps, with the name suffixed `" (copy)"`. The copy is created
/// **disabled** (`enabled = false`) and therefore **not** schedule/app_event
/// trigger-bound — unlike [`flows_create`], which binds a trigger for an
/// enabled flow, this deliberately calls no [`bind_trigger`], so a duplicate
/// can never immediately fire. Run history does not carry over. The user
/// enables it explicitly (via `flows_set_enabled`) once they've reviewed the
/// copy, at which point its trigger binds like any other flow.
pub async fn flows_duplicate(config: &Config, id: &str) -> Result<RpcOutcome<Flow>, String> {
    let source = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    let new_name = format!("{} (copy)", source.name);
    tracing::debug!(target: "flows", source_id = %id, %new_name, "[flows] flows_duplicate: creating disabled, unbound copy");
    let flow =
        store::insert_duplicate_flow(config, &source, new_name).map_err(|e| e.to_string())?;
    // Intentionally NO bind_trigger: a duplicate is disabled and must stay
    // inert (no schedule/trigger dispatch) until the user enables it.
    publish_flow_changed(&flow.id, "created", "system");
    Ok(RpcOutcome::single_log(
        flow,
        format!("flow duplicated from {id}"),
    ))
}

/// Loads one flow by id.
pub async fn flows_get(config: &Config, id: &str) -> Result<RpcOutcome<Flow>, String> {
    let flow = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    Ok(RpcOutcome::single_log(flow, format!("flow loaded: {id}")))
}

/// Loads a saved flow's portable [`WorkflowGraph`] by id, for the
/// `sub_workflow`-by-`workflow_id` resolver capability
/// (`tinyflows::caps::WorkflowResolver`, implemented in
/// `src/openhuman/flows/tinyflows/caps.rs`).
///
/// Returns `Ok(None)` when no flow with that id exists (the resolver turns that
/// into a capability error naming the missing id), and `Err` only on a store
/// failure. Kept sync (the underlying [`store::get_flow`] is sync) so the
/// resolver can call it directly from its async method without a runtime hop.
pub fn load_flow_graph(config: &Config, id: &str) -> Result<Option<WorkflowGraph>, String> {
    tracing::debug!(target: "flows", flow_id = %id, "[flows] load_flow_graph: loading saved flow graph for sub_workflow resolver");
    let graph = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .map(|flow| flow.graph);
    tracing::debug!(
        target: "flows",
        flow_id = %id,
        found = graph.is_some(),
        "[flows] load_flow_graph: resolver lookup complete"
    );
    Ok(graph)
}

/// Resolver-only saved-graph lookup. Authoring tools use [`load_flow_graph`]
/// so a legacy draft can still be opened and repaired; execution resolves only
/// graphs the current engine can run safely.
pub(crate) fn load_engine_compatible_flow_graph(
    config: &Config,
    id: &str,
) -> Result<Option<WorkflowGraph>, String> {
    let graph = load_flow_graph(config, id)?;
    if let Some(graph) = graph.as_ref() {
        ensure_config_aware_engine_compatible(config, graph)
            .map_err(|error| format!("workflow_id '{id}' is engine-incompatible: {error}"))?;
    }
    Ok(graph)
}

/// Lists every saved flow.
///
/// A corrupt or newer-schema-than-this-build `graph_json` row is skipped
/// rather than failing the whole list (R-M4 — see `store::list_flow_rows`);
/// when that happens it must not be silent, so a skip is both logged
/// (`[flows]`-prefixed, id + error only — never row content) and surfaced in
/// the RPC's `logs` so the UI can tell the user "N workflows could not be
/// loaded" instead of silently rendering a shorter list than actually exists.
pub async fn flows_list(config: &Config) -> Result<RpcOutcome<Vec<Flow>>, String> {
    let (flows, skipped) = store::list_flows(config).map_err(|e| e.to_string())?;
    if skipped > 0 {
        tracing::warn!(
            target: "flows",
            skipped,
            loaded = flows.len(),
            "[flows] flows_list: skipped corrupt/unmigratable flow_definitions rows"
        );
        Ok(RpcOutcome::new(
            flows,
            vec![format!(
                "flows listed ({skipped} workflow{} could not be loaded and were skipped)",
                if skipped == 1 { "" } else { "s" }
            )],
        ))
    } else {
        Ok(RpcOutcome::single_log(flows, "flows listed"))
    }
}

/// Lists the connection sources a flow node's `connection_ref` can attach to:
/// Composio connected accounts (`kind = "composio"`) and stored HTTP
/// credentials (`kind = "http"`). This is the picker source for the Workflows
/// UI (and the agent's flow-authoring surface) — it returns ids + display
/// labels + kind ONLY, never any secret material.
///
/// The two sources are aggregated independently and are individually
/// fault-tolerant: a transient Composio backend/network failure (or an
/// unconfigured Direct-mode key) yields zero Composio entries but still returns
/// the HTTP credential half, and vice-versa. A failure in one source never
/// fails the whole picker.
pub async fn flows_list_connections(
    config: &Config,
) -> Result<RpcOutcome<Vec<FlowConnection>>, String> {
    tracing::debug!(
        "[flows] rpc flows_list_connections: aggregating composio + http_cred picker sources"
    );
    let mut logs = Vec::new();

    // 1. Composio connected accounts. Direct mode without a configured key
    //    already short-circuits to an empty list (a valid setup state, not an
    //    error); a backend outage returns Err — tolerate it so the picker still
    //    surfaces HTTP credentials.
    let composio_conns =
        match crate::openhuman::integrations::composio::ops::composio_list_connections(config).await
        {
            Ok(outcome) => {
                tracing::debug!(
                    count = outcome.value.connections.len(),
                    "[flows] flows_list_connections: composio source returned connections"
                );
                outcome.value.connections
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[flows] flows_list_connections: composio source unavailable — \
                     returning http_cred entries only"
                );
                logs.push(format!(
                    "flows_list_connections: composio source unavailable ({e})"
                ));
                Vec::new()
            }
        };

    // 2. Named HTTP credentials — secret-free summaries (the store never hands
    //    out secret material here; injection happens server-side in
    //    `tinyflows::caps::OpenHumanHttp`).
    let http_creds =
        match crate::openhuman::security::credentials::HttpCredentialsStore::from_config(config)
            .list()
        {
            Ok(list) => {
                tracing::debug!(
                    count = list.len(),
                    "[flows] flows_list_connections: http_cred store returned summaries"
                );
                list
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[flows] flows_list_connections: http_cred store read failed — \
                     returning composio entries only"
                );
                logs.push(format!(
                    "flows_list_connections: http_cred store unavailable ({e})"
                ));
                Vec::new()
            }
        };

    // Connected-account identities (email/handle/platform user id), synced
    // via each toolkit's whoami-style call (e.g. Slack `SLACK_TEST_AUTH`) on
    // connection sync. Loaded once here so `build_flow_connections` can stay
    // a pure, unit-testable matcher.
    let identities =
        crate::openhuman::integrations::composio::identity_store::load_connected_identities(
            config,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                %error,
                "[flows] flows_list_connections: load_connected_identities failed"
            );
            Vec::new()
        });
    tracing::debug!(
        count = identities.len(),
        "[flows] flows_list_connections: identity-cache load"
    );
    let connections = build_flow_connections(composio_conns, http_creds, &identities);
    tracing::debug!(
        total = connections.len(),
        "[flows] flows_list_connections: aggregated picker sources"
    );
    logs.push(format!(
        "flows_list_connections: {} connection(s)",
        connections.len()
    ));
    Ok(RpcOutcome::new(connections, logs))
}
