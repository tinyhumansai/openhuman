
fn handle_refresh_all_identities(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_refresh_all_identities(&config).await?)
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        let reason = read_optional::<String>(&params, "reason")?;
        to_json(super::ops::composio_sync(&config, &connection_id, reason).await?)
    })
}

fn handle_get_user_scopes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let toolkit = match read_required_non_empty(&params, "toolkit") {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    method = "composio.get_user_scopes",
                    error = %e,
                    "[composio:scopes] missing required `toolkit` param"
                );
                return Err(e);
            }
        };
        tracing::debug!(
            method = "composio.get_user_scopes",
            toolkit = %toolkit,
            "[composio:scopes] handler entry"
        );
        // Reads through the bound memory driver's `Graph` family, not through
        // the engine's `load_or_default` (which resolved the in-process client
        // itself — openhuman#5560 deleted that client). The read still fails
        // OPEN onto the default pref for every failure mode, deliberately; see
        // `ops::user_scopes`.
        //
        // The config load is `?`-free for the same reason: this handler has
        // never had a failing path, and turning "settings file momentarily
        // unreadable" into an RPC error would make the scopes panel fail where
        // it used to render the defaults.
        let pref = match config_rpc::load_config_with_timeout().await {
            Ok(config) => super::ops::load_user_scope_pref(&config, &toolkit).await,
            Err(error) => {
                tracing::warn!(
                    method = "composio.get_user_scopes",
                    toolkit = %toolkit,
                    %error,
                    "[composio:scopes] config load failed, using default pref (read+write)"
                );
                super::providers::UserScopePref::default()
            }
        };
        tracing::debug!(
            method = "composio.get_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler exit"
        );
        to_json(crate::rpc::RpcOutcome::new(pref, vec![]))
    })
}

fn handle_set_user_scopes(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let toolkit = match read_required_non_empty(&params, "toolkit") {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    method = "composio.set_user_scopes",
                    error = %e,
                    "[composio:scopes] missing required `toolkit` param"
                );
                return Err(e);
            }
        };
        let read: bool = read_required(&params, "read")?;
        let write: bool = read_required(&params, "write")?;
        let admin: bool = read_required(&params, "admin")?;
        let pref = super::providers::UserScopePref { read, write, admin };
        tracing::debug!(
            method = "composio.set_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler entry"
        );
        // Writes through the bound memory driver's `Graph` family. This half
        // fails CLOSED, as it did before: the old code refused with "memory
        // client not initialised" rather than reporting a save it had not
        // done, and `ops::user_scopes::save` refuses on the same three
        // grounds (no driver, no `Graph` family, backend write failure).
        let config = config_rpc::load_config_with_timeout().await?;
        if let Err(e) = super::ops::save_user_scope_pref(&config, &toolkit, pref).await {
            tracing::error!(
                method = "composio.set_user_scopes",
                toolkit = %toolkit,
                error = %e,
                "[composio:scopes] save failed"
            );
            return Err(e);
        }
        tracing::debug!(
            method = "composio.set_user_scopes",
            toolkit = %toolkit,
            read = pref.read,
            write = pref.write,
            admin = pref.admin,
            "[composio:scopes] handler exit"
        );
        to_json(crate::rpc::RpcOutcome::new(pref, vec![]))
    })
}

fn handle_list_available_triggers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: ListAvailableTriggersParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let toolkit = payload.toolkit.trim();
        if toolkit.is_empty() {
            return Err("invalid params: 'toolkit' must not be empty".to_string());
        }
        to_json(
            super::ops::composio_list_available_triggers(&config, toolkit, payload.connection_id)
                .await?,
        )
    })
}

fn handle_list_triggers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: ListTriggersParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(super::ops::composio_list_triggers(&config, payload.toolkit).await?)
    })
}

fn handle_enable_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: EnableTriggerParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let connection_id = payload.connection_id.trim();
        let slug = payload.slug.trim();
        if connection_id.is_empty() {
            return Err("invalid params: 'connection_id' must not be empty".to_string());
        }
        if slug.is_empty() {
            return Err("invalid params: 'slug' must not be empty".to_string());
        }
        to_json(
            super::ops::composio_enable_trigger(
                &config,
                connection_id,
                slug,
                payload.trigger_config,
            )
            .await?,
        )
    })
}

fn handle_disable_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let trigger_id = read_required_non_empty(&params, "trigger_id")?;
        to_json(super::ops::composio_disable_trigger(&config, &trigger_id).await?)
    })
}

fn handle_get_mode(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[composio-direct] rpc get_mode entry");
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_get_mode(&config).await?)
    })
}

fn handle_set_api_key(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[composio-direct] rpc set_api_key entry");
        let config = config_rpc::load_config_with_timeout().await?;
        let api_key = read_required_non_empty(&params, "api_key")?;
        let activate_direct = read_optional::<bool>(&params, "activate_direct")?.unwrap_or(false);
        to_json(super::ops::composio_set_api_key(&config, &api_key, activate_direct).await?)
    })
}

fn handle_clear_api_key(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[composio-direct] rpc clear_api_key entry");
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_clear_api_key(&config).await?)
    })
}

// ── Param helpers ───────────────────────────────────────────────────

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

/// Read a required `String` parameter and reject blank / whitespace-only
/// input at the RPC boundary instead of letting it reach the backend.
/// Returns the trimmed value.
fn read_required_non_empty(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    let raw = read_required::<String>(params, key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("'{key}' must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
