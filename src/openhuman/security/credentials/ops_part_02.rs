
async fn fetch_current_user_for_session_store_inner(
    client: &BackendOAuthClient,
    token: &str,
) -> Result<Value, String> {
    match client.fetch_current_user(token).await {
        Ok(user) => Ok(user),
        Err(first) => {
            let first_reason = format!("{first:#}");
            if !auth_me_store_failure_is_transient(&first_reason) {
                return Err(first_reason);
            }

            tokio::time::sleep(AUTH_ME_STORE_RETRY_DELAY).await;
            tracing::debug!(
                domain = "credentials",
                operation = "fetch_current_user_for_session_store",
                reason = %first_reason,
                "[credentials][auth-store] retrying GET /auth/me after transient failure"
            );
            client
                .fetch_current_user(token)
                .await
                .map_err(|second| format!("{second:#}"))
        }
    }
}

fn auth_me_store_failure_is_transient(reason: &str) -> bool {
    if let Some(status) = auth_me_failure_status(reason) {
        return AUTH_ME_STORE_TRANSIENT_STATUSES.contains(&status);
    }

    crate::core::observability::contains_transient_transport_phrase(reason)
}

fn auth_me_failure_status(reason: &str) -> Option<u16> {
    let lower = reason.to_ascii_lowercase();
    (100..600).find(|status| {
        let status = status.to_string();
        lower.contains(&format!("({status}"))
            || lower.contains(&format!("http {status}"))
            || lower.contains(&format!("status {status}"))
            || lower.contains(&format!("status code {status}"))
    })
}

fn jwt_exp_live_at(
    token: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let exp = decode_jwt_exp(token)?;
    (exp > now).then_some(exp)
}

fn fallback_session_user_for_deferred_validation() -> Value {
    json!({ "pendingBackendValidation": true })
}

fn sanitize_stored_session_user(user: Option<serde_json::Value>) -> Option<serde_json::Value> {
    match user {
        Some(serde_json::Value::Object(map)) if map.is_empty() => None,
        Some(serde_json::Value::Null) => None,
        other => other,
    }
}

fn normalize_local_session_user(user: serde_json::Value, local_user_id: &str) -> serde_json::Value {
    let mut map = match user {
        serde_json::Value::Object(map) => map,
        other => return other,
    };
    map.insert(
        "id".to_string(),
        serde_json::Value::String(local_user_id.to_string()),
    );
    map.insert(
        "_id".to_string(),
        serde_json::Value::String(local_user_id.to_string()),
    );
    serde_json::Value::Object(map)
}

pub async fn clear_session(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut logs = Vec::new();
    // Flip the scheduler-gate override first so any background worker that
    // is mid-iteration (or wakes up while we tear down) stalls at its next
    // `wait_for_capacity()` call instead of firing requests at a backend
    // we're about to invalidate. Idempotent.
    crate::openhuman::cron::scheduler_gate::set_signed_out(true);

    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;

    // The core process stays alive on logout. Tear down its authenticated
    // Socket.IO transport and the user-pinned workflow bridge so neither can
    // keep serving the signed-out account until a later reconnect.
    if let Some(manager) = crate::openhuman::platform::socket::global_socket_manager() {
        if let Err(error) = manager.disconnect().await {
            tracing::warn!(%error, "failed to disconnect backend socket on logout");
        }
    }
    crate::openhuman::platform::socket::medulla::workflows::clear_workflow_bridge();

    // Clear the active user marker so subsequent config loads fall back to the
    // default (unauthenticated) openhuman directory.
    if let Ok(root_dir) = default_root_openhuman_dir() {
        if let Err(e) = crate::openhuman::config::clear_active_user(&root_dir) {
            tracing::warn!(error = %e, "failed to clear active_user.toml on logout");
        }
    }

    // Stop all login-gated services (voice and local AI) so
    // they don't run as orphan processes after logout, consuming RAM/CPU with
    // no user context to operate against.
    stop_login_gated_services(config).await;

    // The process stays alive after desktop/TUI logout, so every process-global
    // store must follow the now-active pre-login workspace. Without this, a
    // signed-out caller can keep reading the previous account's context until
    // the process restarts.
    match crate::openhuman::config::load_config_with_timeout().await {
        Ok(signed_out_config) => {
            let workspace = signed_out_config.workspace_dir.clone();
            // No `memory::global::init` twin here either — see the login site.
            // The context rebind below carries the signed-out workspace and its
            // `[subsystems.memory]` block, and the memory binding is keyed on
            // exactly that pair, so it follows without a second call.
            if let Err(error) = crate::core::runtime::context::CoreContext::rebind_default_workspace(
                &workspace,
                signed_out_config.subsystems.memory.clone(),
            ) {
                tracing::warn!(%error, "failed to rebind core context after logout");
            }
            crate::openhuman::memory::conversations::register_conversation_persistence_subscriber(
                workspace.clone(),
            );
            logs.push(format!(
                "process globals rebound to signed-out workspace {}",
                workspace.display()
            ));
        }
        Err(error) => {
            tracing::warn!(%error, "failed to resolve signed-out workspace after logout");
            logs.push(format!("signed-out workspace rebind warning: {error}"));
        }
    }

    // Drop the Sentry scope user so events surfaced during/after teardown
    // (and before the next login) are no longer attributed to the
    // signed-out account — issue #3135.
    super::sentry_scope::clear();

    logs.push("session cleared".to_string());
    Ok(RpcOutcome::new(json!({ "removed": removed }), logs))
}

pub async fn auth_get_state(
    config: &Config,
) -> Result<RpcOutcome<super::responses::AuthStateResponse>, String> {
    let state = build_session_state(config)?;
    Ok(RpcOutcome::single_log(state, "session state fetched"))
}

pub async fn auth_get_session_token_json(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let token = get_session_token(config)?;
    Ok(RpcOutcome::single_log(
        json!({ "token": token }),
        "session token fetched",
    ))
}

pub async fn auth_get_me(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let user = client
        .fetch_current_user(&token)
        .await
        // `flatten_authed_error` maps the typed `BackendApiError::Unauthorized`
        // onto the `SESSION_EXPIRED:` sentinel and falls through to `{e:#}` for
        // everything else, so both properties this call site needs are kept:
        //
        // * Non-401s still render the full anyhow context chain, so the
        //   underlying reqwest transport error (timeout / connection reset /
        //   TLS / DNS) reaches `observability::is_transient_message_failure`.
        //   Bare `e.to_string()` renders only the top context layer
        //   ("GET /auth/me") and collapsed every transient transport failure
        //   into Sentry TAURI-RUST-10.
        // * A 401 is recognised by `jsonrpc::is_session_expired_error`, which
        //   skips the Sentry report AND publishes `DomainEvent::SessionExpired`
        //   so `SessionExpiredSubscriber` clears the dead JWT.
        //
        // Until #5232 routed `fetch_current_user` through `authed_json`, a 401
        // here surfaced as `"GET /auth/me failed (401 Unauthorized): …"`, which
        // `is_session_expired_error` matched on its HTTP-verb prefix. The typed
        // error renders as `"backend rejected session token on GET /auth/me"`,
        // which matches neither classifier — so on 0.63.9 every lapsed session
        // reported to Sentry as a code defect (TAURI-RUST-RYD) and, because the
        // stale token was never cleared, re-fired the same 401 on the next
        // revalidation: the forced sign-out loop in #5307.
        .map_err(crate::api::flatten_authed_error)?;

    Ok(RpcOutcome::single_log(user, "current user fetched"))
}

pub async fn consume_login_token(
    config: &Config,
    login_token: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let token = login_token.trim();
    if token.is_empty() {
        return Err("loginToken is required".to_string());
    }

    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let jwt_token = client
        .consume_login_token(token)
        .await
        // See `auth_get_me` above for why we walk the full anyhow chain.
        .map_err(|e| format!("{e:#}"))?;

    Ok(RpcOutcome::new(
        serde_json::json!({ "jwtToken": jwt_token }),
        vec![
            format!(
                "login token consumed via POST /auth/login-token/consume on {}",
                api_url.trim_end_matches('/')
            ),
            "session JWT received".to_string(),
        ],
    ))
}

pub async fn auth_create_channel_link_token(
    config: &Config,
    channel: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let channel = channel.trim();
    if channel.is_empty() {
        return Err("channel is required".to_string());
    }
    let channel = channel.to_lowercase();
    if !matches!(channel.as_str(), "telegram" | "discord") {
        return Err(format!("unsupported channel: {channel}"));
    }

    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let payload = client
        .create_channel_link_token(&channel, &token)
        .await
        // See `auth_get_me` above: same authed backend route, same need to keep
        // the typed 401 classifiable while non-401s keep their full anyhow chain.
        .map_err(crate::api::flatten_authed_error)?;

    Ok(RpcOutcome::single_log(
        payload,
        "channel link token created",
    ))
}

pub async fn store_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
    token: Option<String>,
    fields: Option<serde_json::Value>,
    set_active: Option<bool>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let provider = provider.trim().to_string();
    if provider.is_empty() {
        return Err("provider is required".to_string());
    }

    let profile_name = profile_name_or_default(profile);
    let mut metadata = parse_fields_value(fields)?;
    let token = token
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| metadata.get("token").cloned())
        .or_else(|| metadata.get("api_key").cloned())
        .unwrap_or_default();
    if token.is_empty() && metadata.is_empty() {
        return Err("provide at least one credential via token or fields".to_string());
    }
    metadata.remove("token");

    let auth = AuthService::from_config(config);
    let stored = auth
        .store_provider_token(
            &provider,
            profile_name,
            &token,
            metadata,
            set_active.unwrap_or(true),
        )
        .map_err(|e| e.to_string())?;
    // A freshly-stored key supersedes any prior auth rejection for this
    // provider — clear the recorded BYO auth error so the AI-settings notice
    // disappears and the notification latch re-arms (a future rejection will
    // notify again). Credentials are keyed `provider:<slug>`; the auth-error
    // registry is keyed by the bare provider slug used by the chat factory.
    clear_provider_auth_error(&provider);
    Ok(RpcOutcome::single_log(
        summarize_auth_profile(&stored),
        "provider credentials stored",
    ))
}

/// Clear any recorded BYO provider auth error for a credentials `provider`
/// key. Strips the `provider:` namespace prefix so the lookup matches the
/// bare slug (`openrouter`) the inference classifier records under.
fn clear_provider_auth_error(provider: &str) {
    let slug = provider.strip_prefix("provider:").unwrap_or(provider);
    crate::openhuman::inference::auth_error_registry::clear(slug);
}

pub async fn remove_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let profile_name = profile_name_or_default(profile);
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(provider, profile_name)
        .map_err(|e| e.to_string())?;
    // Removing the key clears any recorded BYO auth error for this provider —
    // there is no longer a key to be "rejected", so the stale notice must go.
    clear_provider_auth_error(provider);
    Ok(RpcOutcome::single_log(
        json!({
            "removed": removed,
            "provider": provider,
            "profile": profile_name,
        }),
        "provider credentials removed",
    ))
}

pub async fn list_provider_credentials(
    config: &Config,
    provider_filter: Option<String>,
) -> Result<RpcOutcome<Vec<super::responses::AuthProfileSummary>>, String> {
    let auth = AuthService::from_config(config);
    let profiles = auth.load_profiles().map_err(|e| e.to_string())?;
    let mut items = profiles
        .profiles
        .values()
        .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
        .filter(|profile| {
            provider_filter
                .as_ref()
                .is_none_or(|provider| profile.provider == *provider)
        })
        .map(summarize_auth_profile)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.profile_name.cmp(&b.profile_name))
    });

    Ok(RpcOutcome::single_log(items, "provider credentials listed"))
}

/// List credentials whose provider key starts with `prefix`.
///
/// Pure prefix variant of [`list_provider_credentials`] for namespaces
/// that group multiple providers under a common stem (e.g.
/// `"channel:"` covers `channel:telegram:managed_dm`,
/// `channel:slack:bot_token`, …). The exact-match filter on
/// `list_provider_credentials` cannot express this without enumerating
/// every concrete provider key up front.
pub async fn list_provider_credentials_by_prefix(
    config: &Config,
    prefix: &str,
) -> Result<Vec<super::responses::AuthProfileSummary>, String> {
    let auth = AuthService::from_config(config);
    let profiles = auth.load_profiles().map_err(|e| e.to_string())?;
    let mut items = profiles
        .profiles
        .values()
        .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
        .filter(|profile| profile.provider.starts_with(prefix))
        .map(summarize_auth_profile)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.profile_name.cmp(&b.profile_name))
    });
    Ok(items)
}

pub async fn oauth_connect(
    config: &Config,
    provider: &str,
    skill_id: Option<&str>,
    response_type: Option<&str>,
    encryption_mode: Option<&str>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| {
        "session JWT required; complete login and store_session first".to_string()
    })?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let r = client
        .connect(provider, &token, skill_id, response_type, encryption_mode)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "oauthUrl": r.oauth_url, "state": r.state }),
        "oauth connect URL ready",
    ))
}

pub async fn oauth_list_integrations(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let list = client
        .list_integrations(&token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&list).map_err(|e| e.to_string())?,
        "integrations listed",
    ))
}

pub async fn oauth_fetch_integration_tokens(
    config: &Config,
    integration_id: &str,
    encryption_key: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let tokens = client
        .fetch_integration_tokens_handoff(integration_id, &token, encryption_key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&tokens).map_err(|e| e.to_string())?,
        "integration tokens retrieved",
    ))
}

pub async fn oauth_fetch_client_key(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let client_key = client
        .fetch_client_key(integration_id, &token)
        .await
        .map_err(|e| e.to_string())?;
    log::debug!(
        "[credentials] client key retrieved for integration {}",
        integration_id
    );
    Ok(RpcOutcome::single_log(
        json!({ "clientKey": client_key, "integrationId": integration_id }),
        "client key retrieved (one-time handoff)",
    ))
}

pub async fn oauth_revoke_integration(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .revoke_integration(integration_id, &token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "revoked": true, "integrationId": integration_id }),
        "integration revoked",
    ))
}

/// Provider slot for the user-provided Composio API key when running in
/// direct mode (BYO key).
///
/// Parallel to [`APP_SESSION_PROVIDER`] but completely independent — the
/// app-session JWT authenticates the user against `api.tinyhumans.ai`,
/// while this slot authenticates the user against
/// `backend.composio.dev`. Stored via the same
/// [`super::profiles::AuthProfilesStore`] backend (encrypted on disk
/// when `secrets.encrypt = true`).
pub const COMPOSIO_DIRECT_PROVIDER: &str = "composio-direct";

/// Persist the user-provided Composio API key to the encrypted credential
/// store under [`COMPOSIO_DIRECT_PROVIDER`].
///
/// **Never log the API key itself** — the debug line below records only
/// length and a length-of-stored marker. This honours the CLAUDE.md
/// debug-logging rule (`Never log secrets … redact or omit`).
pub async fn store_composio_api_key(
    config: &Config,
    api_key: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio api_key must not be empty".to_string());
    }
    tracing::debug!(
        len = trimmed.len(),
        "[composio-direct] storing api key (redacted)"
    );
    let auth = AuthService::from_config(config);
    auth.store_provider_token(
        COMPOSIO_DIRECT_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        trimmed,
        std::collections::HashMap::new(),
        true,
    )
    .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        json!({ "stored": true, "provider": COMPOSIO_DIRECT_PROVIDER }),
        "composio direct api key stored",
    ))
}

/// Read the user-provided Composio API key from the encrypted credential
/// store. Returns `Ok(None)` when no key has been stored yet.
///
/// Used by [`crate::openhuman::integrations::composio::client::create_composio_client`]
/// to decide whether direct mode can actually be activated.
pub fn get_composio_api_key(config: &Config) -> Result<Option<String>, String> {
    let auth = AuthService::from_config(config);
    let key = auth
        .get_provider_bearer_token(COMPOSIO_DIRECT_PROVIDER, None)
        .map_err(|e| e.to_string())?;
    Ok(key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()))
}

/// RPC wrapper around [`store_composio_api_key`] — accepts plain string
/// for symmetry with `store_provider_credentials` while only persisting
/// the trimmed value.
pub async fn rpc_store_composio_api_key(
    config: &Config,
    api_key: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    store_composio_api_key(config, api_key).await
}

/// Remove the stored Composio direct-mode API key. Used when the user
/// switches back to backend mode and explicitly clears their key.
pub async fn clear_composio_api_key(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    tracing::debug!("[composio-direct] clearing stored api key");
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(COMPOSIO_DIRECT_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "removed": removed }),
        "composio direct api key cleared",
    ))
}
