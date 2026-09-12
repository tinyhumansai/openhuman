
fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::RemoveRequest>(Value::Object(params))?;
        to_json(rpc::remove_rpc(req).await?)
    })
}

fn handle_list_items(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ListItemsRequest>(Value::Object(params))?;
        to_json(rpc::list_items_rpc(req).await?)
    })
}

fn handle_read_item(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ReadItemRequest>(Value::Object(params))?;
        to_json(rpc::read_item_rpc(req).await?)
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::SyncRequest>(Value::Object(params))?;
        to_json(rpc::sync_rpc(req).await?)
    })
}

fn handle_reconcile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ReconcileRequest>(Value::Object(params))?;
        to_json(rpc::reconcile_rpc(req).await?)
    })
}

fn handle_status_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::status_list_rpc().await?) })
}

fn handle_supported_toolkits(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::supported_toolkits_rpc().await?) })
}

fn handle_sync_audit_log(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::sync_audit_log_rpc().await?) })
}

fn handle_estimate_sync_cost(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::EstimateSyncCostRequest>(Value::Object(params))?;
        to_json(rpc::estimate_sync_cost_rpc(req).await?)
    })
}

fn handle_monthly_cost_summary(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::monthly_cost_summary_rpc().await?) })
}

fn handle_apply_all_in(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::apply_all_in_rpc().await?) })
}

fn handle_coding_session_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::coding_session_status_rpc().await?) })
}

fn handle_ingest_coding_sessions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // `rpc::CodingSessionIngestRequest`, not the engine path: this adapter
        // names its own domain's request type, the way every sibling here does.
        // See the re-export's docs in `rpc.rs` for why the type is still the
        // engine's underneath (#5560).
        let req = parse_value::<rpc::CodingSessionIngestRequest>(Value::Object(params))?;
        to_json(rpc::ingest_coding_sessions_rpc(req).await?)
    })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
