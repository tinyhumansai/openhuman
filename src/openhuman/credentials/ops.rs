//! JSON-RPC / CLI controller surface for credentials and app session auth.

use serde_json::json;

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::{user_id_from_profile_payload, BackendOAuthClient};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::{
    build_session_state, parse_fields_value, profile_name_or_default, summarize_auth_profile,
};
use crate::openhuman::security::SecretStore;
use crate::rpc::RpcOutcome;

use super::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use crate::openhuman::config::{
    default_root_openhuman_dir, pre_login_user_dir, read_active_user_id, user_openhuman_dir,
    write_active_user_id,
};
use crate::openhuman::memory::conversations;

/// Start all login-gated background services (local AI, voice, screen
/// intelligence, autocomplete).  Called both from the initial boot path
/// (when an existing session is detected) and from `store_session()` on
/// fresh login.
pub async fn start_login_gated_services(config: &Config) {
    // 1. Local AI (Ollama, whisper, embeddings)
    if config.local_ai.runtime_enabled {
        let service = crate::openhuman::local_ai::global(config);
        service.bootstrap(config).await;
        log::info!("[services] local AI bootstrapped after login");
    }

    // 2. Voice server (records + transcribes via hotkey)
    crate::openhuman::voice::server::start_if_enabled(config).await;

    // 3. Dictation hotkey listener (only when voice server is NOT auto-started,
    //    since the voice server owns the single rdev listener on macOS)
    if !config.voice_server.auto_start {
        crate::openhuman::voice::dictation_listener::start_if_enabled(config).await;
    }

    // 4. Screen intelligence (capture + vision analysis)
    crate::openhuman::screen_intelligence::server::start_if_enabled(config).await;

    // 5. Autocomplete (text suggestions + Swift overlay helper)
    crate::openhuman::autocomplete::start_if_enabled(config).await;

    log::info!("[services] all login-gated services started");
}

/// Stop all login-gated background services.  Called from `clear_session()`
/// on logout so orphan processes don't consume resources.
pub async fn stop_login_gated_services(config: &Config) {
    // 1. Autocomplete — stop engine + Swift overlay helper.
    {
        let engine = crate::openhuman::autocomplete::global_engine();
        let status = engine.status().await;
        if status.running {
            engine.stop(None).await;
            log::info!("[services] autocomplete engine stopped on logout");
        }
    }

    // 2. Voice server
    if let Some(server) = crate::openhuman::voice::server::try_global_server() {
        server.stop().await;
        log::info!("[services] voice server stopped on logout");
    }

    // 3. Screen intelligence server
    if let Some(server) = crate::openhuman::screen_intelligence::server::try_global_server() {
        server.stop().await;
        log::info!("[services] screen intelligence server stopped on logout");
    }

    // 4. Local AI — reset state to idle. We don't kill the Ollama process
    //    (it may be serving other clients or mid-download), but we clear
    //    the internal state so it re-bootstraps on next login.
    if config.local_ai.runtime_enabled {
        let service = crate::openhuman::local_ai::global(config);
        service.reset_to_idle(config);
        log::info!("[services] local AI reset to idle on logout");
    }

    // 5. Dictation listener — abort the hotkey forwarder task so it doesn't
    //    accumulate duplicate rdev listeners across logout → login cycles.
    crate::openhuman::voice::dictation_listener::stop();

    log::info!("[services] all login-gated services stopped");
}

fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

pub async fn encrypt_secret(
    config: &Config,
    plaintext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let ciphertext = store.encrypt(plaintext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(ciphertext, "secret encrypted"))
}

pub async fn decrypt_secret(
    config: &Config,
    ciphertext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let plaintext = store.decrypt(ciphertext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(plaintext, "secret decrypted"))
}

pub async fn store_session(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<serde_json::Value>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let trimmed_token = token.trim();
    if trimmed_token.is_empty() {
        return Err("token is required".to_string());
    }

    let api_url = effective_api_url(&config.api_url);

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let settings = client
        .fetch_current_user(trimmed_token)
        .await
        .map_err(|e| format!("Session validation failed (GET /auth/me): {e:#}"))?;

    let mut metadata = std::collections::HashMap::new();
    if let Some(uid) = user_id
        .and_then(|v| {
            let t = v.trim().to_string();
            (!t.is_empty()).then_some(t)
        })
        .or_else(|| user_id_from_profile_payload(&settings))
    {
        metadata.insert("user_id".to_string(), uid);
    }
    let user_for_store = sanitize_stored_session_user(user).unwrap_or(settings);
    metadata.insert("user_json".to_string(), user_for_store.to_string());

    // Determine user_id so we can scope the openhuman directory to this user.
    let resolved_user_id = metadata.get("user_id").cloned();

    // If we know the user_id, activate the user-scoped directory BEFORE storing
    // the auth profile so that credentials land in the correct place.
    let mut logs = vec![format!(
        "session JWT verified via GET /auth/me on {}",
        api_url.trim_end_matches('/')
    )];

    if let Some(ref uid) = resolved_user_id {
        if let Ok(root_dir) = default_root_openhuman_dir() {
            // Snapshot before we overwrite `active_user.toml` so we can tell
            // first activation from signed-out vs an in-place account switch.
            let previous_active = read_active_user_id(&root_dir);
            let user_dir = user_openhuman_dir(&root_dir, uid);
            if let Err(e) = std::fs::create_dir_all(&user_dir) {
                tracing::warn!(
                    user_id = %uid,
                    error = %e,
                    "failed to create user directory"
                );
            } else if let Err(e) = write_active_user_id(&root_dir, uid) {
                tracing::warn!(
                    user_id = %uid,
                    error = %e,
                    "failed to write active_user.toml"
                );
            } else {
                logs.push(format!("user directory activated for {uid}"));
                tracing::info!(
                    user_id = %uid,
                    user_dir = %user_dir.display(),
                    "User-scoped directory activated"
                );
                // Onboarding and other pre-auth flows write threads under the
                // `users/local/workspace` tree. After the first successful login
                // there was no previous `active_user.toml`, wipe that anonymous
                // conversation store so a fresh account never inherits demo or
                // scratch threads from the pre-login bucket (#1157).
                //
                // This shares `memory::conversations`' process-wide mutex with
                // `list_threads` / `purge_threads` on any workspace, so purge and
                // concurrent thread RPC in this process cannot interleave.
                if previous_active.is_none() {
                    let pre_ws = pre_login_user_dir(&root_dir).join("workspace");
                    let pre_ws_log = pre_ws.display().to_string();
                    match conversations::purge_threads(pre_ws) {
                        Ok(stats) => {
                            tracing::info!(
                                pre_login_workspace = %pre_ws_log,
                                threads = stats.thread_count,
                                messages = stats.message_count,
                                "[credentials] purged pre-login conversation threads after first session activation"
                            );
                            logs.push(format!(
                                "purged pre-login conversation history (threads={}, messages={})",
                                stats.thread_count, stats.message_count
                            ));
                        }
                        Err(e) => {
                            tracing::debug!(
                                error = %e,
                                pre_login_workspace = %pre_ws_log,
                                "[credentials] pre-login conversation purge skipped (non-fatal)"
                            );
                        }
                    }
                }
            }
        }
    }

    // Reload config so it picks up the newly activated user directory.
    // This ensures auth-profiles.json, encryption key, etc. are written
    // to the user-scoped location.
    let effective_config = if resolved_user_id.is_some() {
        match crate::openhuman::config::load_config_with_timeout().await {
            Ok(c) => c,
            Err(_) => config.clone(),
        }
    } else {
        config.clone()
    };

    let auth = AuthService::from_config(&effective_config);
    let profile = auth
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            trimmed_token,
            metadata,
            true,
        )
        .map_err(|e| e.to_string())?;

    logs.push("session stored".to_string());

    // Now that active_user.toml exists and config.workspace_dir resolves to
    // the per-user path, seed the subconscious defaults and spawn the
    // heartbeat loop. Idempotent — no-op on subsequent logins of the same
    // process. Bootstrap failures are non-fatal: the session itself is
    // already stored above, so we only warn.
    if let Err(e) = crate::openhuman::subconscious::global::bootstrap_after_login().await {
        tracing::warn!(error = %e, "[subconscious] post-login bootstrap failed");
        logs.push(format!("subconscious bootstrap warning: {e}"));
    } else {
        logs.push("subconscious engine bootstrapped".to_string());
    }

    // Start all login-gated services (voice, autocomplete, screen
    // intelligence, local AI). Uses the effective config so services see
    // the user-scoped workspace directory.
    start_login_gated_services(&effective_config).await;
    logs.push("login-gated services started".to_string());

    // Clear the scheduler-gate signed-out override now that a fresh JWT is
    // in place. Workers that were sleeping in the paused poll loop will
    // pick this up at their next iteration and resume LLM-bound work.
    crate::openhuman::scheduler_gate::set_signed_out(false);

    Ok(RpcOutcome::new(summarize_auth_profile(&profile), logs))
}

fn sanitize_stored_session_user(user: Option<serde_json::Value>) -> Option<serde_json::Value> {
    match user {
        Some(serde_json::Value::Object(map)) if map.is_empty() => None,
        Some(serde_json::Value::Null) => None,
        other => other,
    }
}

pub async fn clear_session(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    // Flip the scheduler-gate override first so any background worker that
    // is mid-iteration (or wakes up while we tear down) stalls at its next
    // `wait_for_capacity()` call instead of firing requests at a backend
    // we're about to invalidate. Idempotent.
    crate::openhuman::scheduler_gate::set_signed_out(true);

    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;

    // Clear the active user marker so subsequent config loads fall back to the
    // default (unauthenticated) openhuman directory.
    if let Ok(root_dir) = default_root_openhuman_dir() {
        if let Err(e) = crate::openhuman::config::clear_active_user(&root_dir) {
            tracing::warn!(error = %e, "failed to clear active_user.toml on logout");
        }
    }

    // Stop all login-gated services (voice, autocomplete, screen
    // intelligence, local AI) so they don't run as orphan processes after
    // logout, consuming RAM/CPU with no user context to operate against.
    stop_login_gated_services(config).await;

    // Tear down the subconscious engine + heartbeat loop. Without this the
    // cached engine would keep pointing at the previous user's workspace_dir
    // and the heartbeat task would leak, ticking against the wrong DB when a
    // different user signs in to the same sidecar process.
    crate::openhuman::subconscious::global::reset_engine_for_user_switch().await;

    Ok(RpcOutcome::single_log(
        json!({ "removed": removed }),
        "session cleared",
    ))
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
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let user = client
        .fetch_current_user(&token)
        .await
        .map_err(|e| e.to_string())?;

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

    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let jwt_token = client
        .consume_login_token(token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::new(
        serde_json::json!({ "jwtToken": jwt_token }),
        vec![
            format!(
                "login token consumed via POST /telegram/login-tokens/:token/consume on {}",
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

    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let payload = client
        .create_channel_link_token(&channel, &token)
        .await
        .map_err(|e| e.to_string())?;

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
    Ok(RpcOutcome::single_log(
        summarize_auth_profile(&stored),
        "provider credentials stored",
    ))
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
    let api_url = effective_api_url(&config.api_url);
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
    let api_url = effective_api_url(&config.api_url);
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
    let api_url = effective_api_url(&config.api_url);
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
    let api_url = effective_api_url(&config.api_url);
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
    let api_url = effective_api_url(&config.api_url);
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

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
