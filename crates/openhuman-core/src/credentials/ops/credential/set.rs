use super::*;
use crate::config::user_scope::{
    activate_user_scope, rebind_after_credential_change, reload_config_or,
};
use crate::core::observability::sentry_scope;
use crate::credentials::session_state::{
    build_session_state, load_app_session_profile, session_token_from_profile, CredentialKind,
    SESSION_EXPIRES_AT_META,
};
use crate::credentials::{
    api_key, identity, AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};

async fn rollback_failed_credential_rebind(auth: &AuthService, cause: &str) -> String {
    if let Err(rollback) = auth
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .await
    {
        return format!("credential rebind failed: {cause}; rollback failed: {rollback}");
    }
    if let Err(rollback) = crate::config::user_scope::deactivate_user_scope() {
        return format!(
            "credential rebind failed: {cause}; active-user rollback failed: {rollback}"
        );
    }
    cause.to_string()
}

/// Install a backend credential. See the module docs for what the core does
/// with it; see [`SetCredentialRequest`] for what the host must supply.
///
/// Calling it again with the **same token and user** is a cheap refresh: only
/// the stored user payload and expiry are rewritten (no directory activation,
/// no purge, no service restart). A **different user** than the one stored is
/// signed out first so `active_user.toml` never points at one user while
/// another's profile exists.
pub async fn set_credential(
    ctx: &DispatchCtx,
    config: &Config,
    request: SetCredentialRequest,
) -> Result<Outcome<AuthStateResponse>, String> {
    refuse_process_credential_in_saas("set_credential")?;
    let issuing_backend = request.issuing_backend.clone();
    let resolved = resolve(request)?;
    let issuing_backend = if resolved.kind == CredentialKind::Session {
        issuing_backend
            .as_deref()
            .map(crate::credentials::session_state::normalize_session_backend)
            .transpose()?
    } else {
        None
    };
    let _mutation = CREDENTIAL_MUTATION_LOCK.lock().await;

    if resolved.kind == CredentialKind::ApiKey {
        let was_authenticated =
            crate::credentials::backend_credential::has_backend_credential(config).await;
        api_key::store_api_key(config, &resolved.token)
            .await
            .map_err(|e| e.to_string())?;
        crate::integrations::composio::invalidate_connected_integrations_cache();
        // API-key-backed runs must not retain a previous session identity in
        // prompt composition or observability scope.
        identity::clear_current_user();
        sentry_scope::clear();
        if !was_authenticated {
            start_credential_gated_services(ctx, config).await;
        } else {
            spawn_integrations_cache_warm(config);
        }
        crate::platform::llm_gate::set_signed_out(false);
        #[cfg(feature = "channels")]
        crate::channels::session::resume_channel_service();
        tracing::info!(
            domain = "credentials",
            operation = "set_credential",
            "{LOG_PREFIX} api key stored"
        );
        let state = build_session_state(config).await?;
        publish_credential_changed(ctx, CredentialKind::ApiKey.as_str());
        return Ok(Outcome::single_log(state, "api key stored"));
    }

    let user_id = resolved
        .user_id
        .clone()
        .expect("session and local credentials always resolve a user id");
    let mut logs = Vec::new();

    // Refresh vs. install vs. account switch, decided on the stored profile.
    let existing = load_app_session_profile(config).await?;
    let existing_token = session_token_from_profile(existing.as_ref());
    let existing_user_id = existing
        .as_ref()
        .and_then(|p| p.metadata.get("user_id").cloned());
    let same_token = existing_token.as_deref() == Some(resolved.token.as_str());
    let same_user = existing_user_id.as_deref() == Some(user_id.as_str());
    let refresh = same_token && same_user;

    let mut metadata = std::collections::HashMap::new();
    if resolved.kind == CredentialKind::Session {
        let previous_backend = existing
            .as_ref()
            .filter(|_| same_token)
            .and_then(|profile| {
                profile
                    .metadata
                    .get(crate::credentials::session_state::SESSION_ISSUING_BACKEND_META)
            });
        if let (Some(previous), Some(issuing)) = (previous_backend, issuing_backend.as_ref()) {
            if previous != issuing {
                return Err("SESSION_BACKEND_MISMATCH: cannot rebind a stored session to a different backend".into());
            }
        }
        if let Some(backend) = previous_backend.cloned().or(issuing_backend) {
            metadata.insert(
                crate::credentials::session_state::SESSION_ISSUING_BACKEND_META.to_string(),
                backend,
            );
        }
    }
    if existing_token.is_some() && !same_user {
        tracing::info!(
            domain = "credentials",
            operation = "set_credential",
            "{LOG_PREFIX} credential names a different user than the stored one; clearing the previous session first"
        );
        let cleared = clear_session_credential(ctx, config).await?;
        logs.extend(cleared.logs);
    }
    metadata.insert("user_id".to_string(), user_id.clone());
    if let Some(user) = &resolved.user {
        metadata.insert("user_json".to_string(), user.to_string());
    } else if let Some(previous) = existing
        .as_ref()
        .filter(|_| refresh)
        .and_then(|p| p.metadata.get("user_json").cloned())
    {
        // A refresh without a payload keeps the one already stored.
        metadata.insert("user_json".to_string(), previous);
    }
    if let Some(exp) = resolved.expires_at {
        // Recorded so `require_live_session_token` can reject an expired token
        // locally instead of firing a doomed backend 401 (#3297).
        metadata.insert(SESSION_EXPIRES_AT_META.to_string(), exp.to_rfc3339());
    }

    let effective_config = if refresh {
        logs.push("credential refreshed (same secret, same user)".to_string());
        config.clone()
    } else {
        logs.extend(activate_user_scope(ctx, &user_id)?);
        // Reload so auth-profiles.json, the encryption key and the workspace
        // resolve to the user-scoped location before the profile is written.
        let effective = reload_config_or(ctx, config).await?;
        if resolved.kind == CredentialKind::Local {
            match crate::config::ops::set_onboarding_completed(ctx, false).await {
                Ok(_) => {
                    logs.push("onboarding left incomplete for local session setup".to_string())
                }
                Err(error) => logs.push(format!(
                    "onboarding setup warning for local session: {error}"
                )),
            }
            logs.push("local session accepted without backend validation".to_string());
        }
        effective
    };

    let auth = AuthService::from_config(&effective_config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        &resolved.token,
        metadata,
        true,
    )
    .await
    .map_err(|e| e.to_string())?;
    logs.push(format!("{} credential stored", resolved.kind.as_str()));

    if !refresh {
        crate::integrations::composio::invalidate_connected_integrations_cache();
        if let Err(error) =
            rebind_after_credential_change(&effective_config, "credential installed")
        {
            // A retry of the same credential takes the refresh fast-path, so
            // leaving its profile behind would let it skip this required bind.
            return Err(rollback_failed_credential_rebind(&auth, &error).await);
        }
        logs.push("process globals rebound after credential install".to_string());
        start_credential_gated_services(ctx, &effective_config).await;
        logs.push("credential-gated services started".to_string());
    }

    // Open the scheduler gate now that a live credential is in place; workers
    // sleeping in the paused poll loop resume at their next iteration.
    crate::platform::llm_gate::set_signed_out(false);
    #[cfg(feature = "channels")]
    crate::channels::session::resume_channel_service();
    // An API key wins over a session for every backend request. Keep the
    // process identity aligned with that effective credential.
    if api_key::has_api_key(&effective_config).await {
        sentry_scope::clear();
        identity::clear_current_user();
    } else {
        sentry_scope::bind(&user_id);
        identity::set_current_user(resolved.user.clone().or_else(|| {
            existing
                .as_ref()
                .filter(|_| refresh)
                .and_then(|p| p.metadata.get("user_json").cloned())
                .and_then(|raw| serde_json::from_str(&raw).ok())
        }));
    }
    tracing::info!(
        domain = "credentials",
        operation = "set_credential",
        kind = resolved.kind.as_str(),
        refresh,
        "{LOG_PREFIX} credential installed"
    );

    let state = build_session_state(&effective_config).await?;
    publish_credential_changed(ctx, resolved.kind.as_str());
    Ok(Outcome::new(state, logs))
}

#[cfg(test)]
#[path = "../../ops_credential_scope_rollback_tests.rs"]
mod scope_rollback_tests;
