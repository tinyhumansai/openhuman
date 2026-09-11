fn handle_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let name = read_required::<String>(&params, "name")?;
        let graph = read_required::<Value>(&params, "graph")?;
        let require_approval = params
            .get("require_approval")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Opt-in strict mode (F3): run the same author hard-gates an agent save
        // must pass, before persisting. Default off — the human canvas save
        // path stays permissive.
        if params
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            ops::strict_gate(&config, &graph).await?;
        }
        to_json(ops::flows_create(&config, name, graph, require_approval).await?)
    })
}

fn handle_validate(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // No config load: validation is pure (no persistence, no workspace).
        let graph = read_required::<Value>(&params, "graph")?;
        to_json(ops::flows_validate(graph))
    })
}

fn handle_import(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // No config load: import is pure (no persistence, no workspace).
        let graph = read_required::<Value>(&params, "graph")?;
        let format = params
            .get("format")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value::<String>(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'format': {e}"))?;
        to_json(ops::flows_import(graph, format)?)
    })
}

fn handle_duplicate(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_duplicate(&config, id.trim()).await?)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_get(&config, id.trim()).await?)
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::flows_list(&config).await?)
    })
}

fn handle_list_connections(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::flows_list_connections(&config).await?)
    })
}

fn handle_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let name = params
            .get("name")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'name': {e}"))?;
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        let require_approval = params.get("require_approval").and_then(Value::as_bool);
        let expected_version = params
            .get("expected_version")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // Opt-in strict mode (F3): when a new graph is supplied, run the same
        // author hard-gates an agent save must pass, before persisting.
        if params
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(graph_json) = graph.as_ref() {
                ops::strict_gate(&config, graph_json).await?;
            }
        }
        to_json(
            ops::flows_update(
                &config,
                id.trim(),
                name,
                graph,
                require_approval,
                expected_version,
            )
            .await?,
        )
    })
}

fn handle_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_delete(&config, id.trim()).await?)
    })
}

fn handle_set_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let enabled = params
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "missing required param 'enabled'".to_string())?;
        to_json(ops::flows_set_enabled(&config, id.trim(), enabled).await?)
    })
}

fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let input = params.get("input").cloned().unwrap_or(Value::Null);
        let inputs = read_declared_inputs(&params)?;
        to_json(
            ops::flows_run(
                &config,
                id.trim(),
                input,
                inputs,
                crate::openhuman::flows::FlowRunTrigger::Rpc,
            )
            .await?,
        )
    })
}

fn handle_run_detached(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let input = params.get("input").cloned().unwrap_or(Value::Null);
        let inputs = read_declared_inputs(&params)?;
        to_json(
            ops::flows_run_detached(
                &config,
                id.trim(),
                input,
                inputs,
                crate::openhuman::flows::FlowRunTrigger::Rpc,
            )
            .await?,
        )
    })
}

/// Reads the optional `inputs` param — values for the flow's declared workflow
/// inputs, keyed by name.
///
/// Absent or `null` means "supplied nothing", which is valid for a flow whose
/// inputs are all optional or defaulted. A present-but-non-object value is a
/// caller error rejected here, before it reaches `ops`, so the message names the
/// parameter rather than surfacing as a confusing per-input complaint.
fn read_declared_inputs(params: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    match params.get("inputs") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map.clone()),
        Some(other) => Err(format!(
            "param 'inputs' must be an object keyed by declared input name, got {}",
            match other {
                Value::Array(_) => "an array",
                Value::String(_) => "a string",
                Value::Number(_) => "a number",
                Value::Bool(_) => "a boolean",
                _ => "a non-object",
            }
        )),
    }
}

fn handle_resume(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let approvals: Vec<String> = params
            .get("approvals")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid 'approvals': {e}"))?
            .unwrap_or_default();
        let rejections: Vec<String> = params
            .get("rejections")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid 'rejections': {e}"))?
            .unwrap_or_default();
        to_json(
            ops::flows_resume(&config, id.trim(), thread_id.trim(), approvals, rejections).await?,
        )
    })
}

fn handle_cancel_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let run_id = read_required::<String>(&params, "run_id")?;
        to_json(ops::flows_cancel_run(&config, run_id.trim()).await?)
    })
}

fn handle_list_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(20);
        to_json(ops::flows_list_runs(&config, id.trim(), limit).await?)
    })
}

fn handle_list_all_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(100);
        to_json(ops::flows_list_all_runs(&config, limit).await?)
    })
}

fn handle_get_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let run_id = read_required::<String>(&params, "run_id")?;
        to_json(ops::flows_get_run(&config, run_id.trim()).await?)
    })
}

fn handle_prune_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_prune_runs(&config, id.trim()).await?)
    })
}

fn handle_build(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        // Optional streaming target: when the copilot passes its chat `thread_id`
        // the builder turn streams live text/tool/proposal events into that
        // thread (Phase B). Read + strip the transport-only keys before the rest
        // of the object is deserialized into the structured BuilderRequest.
        let stream = read_flow_stream_target(&params);
        // Deserialize the remaining param object into the structured BuilderRequest
        // (mode/instruction/graph/flow_id/run_id/error/failing_node_ids). The
        // stream keys are ignored (BuilderRequest doesn't declare them).
        let req: crate::openhuman::flows::agents::workflow_builder::builder_prompt::BuilderRequest =
            serde_json::from_value(Value::Object(params))
                .map_err(|e| format!("invalid flows.build params: {e}"))?;
        to_json(ops::flows_build(&config, req, stream).await?)
    })
}

fn handle_build_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let request_id = params
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        to_json(ops::flows_build_cancel(thread_id.trim(), request_id.as_deref()).await?)
    })
}

fn handle_discover(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        // Optional streaming target for the Flow Scout run (Phase B) — same
        // `thread_id`/`request_id` convention as `flows.build`.
        let stream = read_flow_stream_target(&params);
        to_json(ops::flows_discover(&config, stream).await?)
    })
}

/// Read the optional `thread_id` / `request_id` streaming params shared by
/// `flows.build` and `flows.discover` into an [`ops::FlowStreamTarget`].
/// Returns `None` (headless run) when no usable `thread_id` is present; a
/// missing `request_id` is filled with a fresh uuid inside `from_params`.
fn read_flow_stream_target(params: &Map<String, Value>) -> Option<ops::FlowStreamTarget> {
    let thread_id = params
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let request_id = params
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    ops::FlowStreamTarget::from_params(thread_id, request_id)
}

fn handle_list_suggestions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(crate::openhuman::flows::SuggestionStatus::from_str_lossy);
        to_json(ops::flows_list_suggestions(&config, status).await?)
    })
}

fn handle_dismiss_suggestion(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_dismiss_suggestion(&config, id.trim()).await?)
    })
}

fn handle_mark_suggestion_built(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_mark_suggestion_built(&config, id.trim()).await?)
    })
}

fn handle_required_connections(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let graph = read_required::<Value>(&params, "graph")?;
        to_json(ops::flows_required_connections(&config, graph).await?)
    })
}

fn handle_approval_manifest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        to_json(ops::flows_approval_manifest(&config, id.as_deref(), graph).await?)
    })
}

fn handle_search_tool_catalog(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let query = read_required::<String>(&params, "query")?;
        let toolkit = params
            .get("toolkit")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(25);
        to_json(ops::flows_search_tool_catalog(&config, query.trim(), toolkit, limit).await?)
    })
}

fn handle_get_tool_contract(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let slug = read_required::<String>(&params, "slug")?;
        to_json(ops::flows_get_tool_contract(&config, slug.trim()).await?)
    })
}

fn handle_get_history(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(20);
        to_json(ops::flows_get_history(&config, id.trim(), limit)?)
    })
}

fn handle_rollback(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let revision_id = read_required::<String>(&params, "revision_id")?;
        let expected_version = params
            .get("expected_version")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        to_json(
            ops::flows_rollback(&config, id.trim(), revision_id.trim(), expected_version).await?,
        )
    })
}

fn handle_draft_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let name = read_required::<String>(&params, "name")?;
        let graph = read_required::<Value>(&params, "graph")?;
        let flow_id = params
            .get("flow_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let origin = params
            .get("origin")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_value(Value::String(s.to_string())).ok())
            .unwrap_or(crate::openhuman::flows::DraftOrigin::Canvas);
        to_json(ops::flows_draft_create(
            &config, flow_id, name, graph, origin,
        )?)
    })
}

fn handle_draft_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_draft_get(&config, id.trim())?)
    })
}

fn handle_draft_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let name = params
            .get("name")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'name': {e}"))?;
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        // A present `flow_id` (even null) re-links the draft; absent leaves it.
        let flow_id = parse_draft_update_flow_id(&params)?;
        to_json(ops::flows_draft_update(
            &config,
            id.trim(),
            name,
            graph,
            flow_id,
        )?)
    })
}

fn handle_draft_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::flows_draft_list(&config)?)
    })
}

fn handle_draft_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_draft_delete(&config, id.trim())?)
    })
}

fn handle_draft_promote(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let require_approval = params.get("require_approval").and_then(Value::as_bool);
        to_json(ops::flows_draft_promote(&config, id.trim(), require_approval).await?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

/// Parses `draft_update`'s `flow_id` param (R-m7). The outer `Option`
/// mirrors `ops::flows_draft_update`'s "present vs absent" contract — absent
/// leaves the draft's existing link untouched; the inner `Option` is the new
/// link (`None` unlinks).
///
/// A present-but-non-string `flow_id` (a number, or an object from a buggy
/// client) is REJECTED rather than silently coerced into `Some(None)` via
/// `Value::as_str()` returning `None` on a type mismatch — that shape used
/// to be indistinguishable from an explicit `flow_id: null` unlink, and
/// `update_draft` treats `Some(None)` as exactly that: unlinking the draft
/// from its flow. A later `draft_promote` then creates a brand-new flow
/// instead of updating the one the caller actually meant.
fn parse_draft_update_flow_id(
    params: &Map<String, Value>,
) -> Result<Option<Option<String>>, String> {
    match params.get("flow_id") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) => {
            let s = s.trim();
            Ok(Some(if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }))
        }
        Some(other) => Err(format!(
            "invalid 'flow_id': expected a string or null, got {other}"
        )),
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
