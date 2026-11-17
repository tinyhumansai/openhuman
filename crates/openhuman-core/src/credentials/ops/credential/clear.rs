use super::*;
use crate::config::user_scope::reload_config_or;
use crate::core::observability::sentry_scope;
use crate::credentials::session_state::{load_app_session_profile, CredentialKind};
use crate::credentials::{api_key, identity};
use serde_json::{json, Value};

/// Remove the stored credential of `kind` — or every credential when `None`.
/// A session (or local) removal runs the sign-out teardown; an API-key removal
/// only closes the scheduler gate when no other credential remains.
pub async fn clear_credential(
    ctx: &DispatchCtx,
    config: &Config,
    kind: Option<CredentialKind>,
) -> Result<Outcome<Value>, String> {
    refuse_process_credential_in_saas("clear_credential")?;
    let _mutation = CREDENTIAL_MUTATION_LOCK.lock().await;
    let mut logs = Vec::new();
    let mut removed_session = false;
    let mut removed_api_key = false;

    // The API key is persisted beside `config`'s state dir, which is the
    // user-scoped directory while a session is active. Deactivating that
    // session moves every process global to the pre-login/signed-out
    // workspace, so a key left in place would be stranded under
    // `users/<id>` while the rest of the process reads `users/local` and
    // reports signed out (#6318). Snapshot it before teardown so it can
    // either be carried forward (a session/local-only clear) or removed
    // outright at its real location (`kind: None` clears everything, but
    // the API-key clear below runs against the post-teardown config and
    // would otherwise miss it).
    let source_api_key = if matches!(
        kind,
        None | Some(CredentialKind::Session) | Some(CredentialKind::Local)
    ) {
        api_key::get_api_key(config)
            .await
            .map_err(|e| e.to_string())?
    } else {
        None
    };

    let mut effective_config = config.clone();
    let mut cleared_source_api_key = false;
    let mut carried_api_key = false;
    if matches!(
        kind,
        None | Some(CredentialKind::Session) | Some(CredentialKind::Local)
    ) {
        let outcome = clear_session_credential(ctx, config).await?;
        removed_session = outcome.value;
        logs.extend(outcome.logs);

        if removed_session {
            // Follow the same process globals the teardown just rebound to,
            // so the api-key checks below agree with `auth.get_state`.
            effective_config = reload_config_or(ctx, config).await?;
            if let Some(key) = source_api_key.as_deref() {
                if matches!(
                    kind,
                    Some(CredentialKind::Session) | Some(CredentialKind::Local)
                ) {
                    // A session/local-only clear preserves the key that was
                    // actually active (the source): carry it forward to the
                    // post-teardown workspace before the source copy is
                    // removed below, overwriting whatever the destination
                    // already held. This RPC is only removing the session —
                    // an unrelated, stale key sitting at the pre-login
                    // workspace must never silently outrank the key that was
                    // the effective credential a moment ago (#6318).
                    api_key::store_api_key(&effective_config, key)
                        .await
                        .map_err(|e| e.to_string())?;
                    logs.push("api key carried forward to the signed-out workspace".to_string());
                    carried_api_key = true;
                }
                // Profile documents are local to their config state directory,
                // even when the process uses a generic storage backend for
                // other domains. Skip source cleanup only when the destination
                // store is literally the same file.
                let shares_store = carried_api_key
                    && crate::credentials::profiles::profiles_file(
                        &crate::credentials::state_dir_from_config(config),
                    ) == crate::credentials::profiles::profiles_file(
                        &crate::credentials::state_dir_from_config(&effective_config),
                    );
                if shares_store {
                    log::debug!("[credentials] api key kept: the profile file is shared");
                } else {
                    // Either the key was just moved to the post-teardown config
                    // above, or `kind` is `None` and it must be removed outright.
                    // `config` here is still the pre-teardown, user-scoped
                    // reference, so this clears the *source* location — never
                    // leave that copy behind, or a later login to the same
                    // account (or an api-key clear against the wrong location)
                    // would silently resurrect it (#6318).
                    cleared_source_api_key = api_key::clear_api_key(config)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    let config = &effective_config;

    if matches!(kind, None | Some(CredentialKind::ApiKey)) {
        removed_api_key = api_key::clear_api_key(config)
            .await
            .map_err(|e| e.to_string())?
            || cleared_source_api_key;
        if removed_api_key {
            crate::integrations::composio::invalidate_connected_integrations_cache();
            logs.push("api key cleared".to_string());
        }
        if !crate::credentials::backend_credential::has_backend_credential(config).await {
            crate::platform::llm_gate::set_signed_out(true);
        } else if removed_api_key {
            // Clearing the preferred API key exposes the surviving session.
            // Restore the session's process identity immediately.
            if let Some(profile) = load_app_session_profile(config).await? {
                if let Some(user_id) = profile.metadata.get("user_id") {
                    sentry_scope::bind(user_id);
                }
                identity::set_current_user(
                    profile
                        .metadata
                        .get("user_json")
                        .and_then(|raw| serde_json::from_str(raw).ok()),
                );
            }
            spawn_integrations_cache_warm(config);
        }
    }

    // A session-only clear may leave an API key behind. The session teardown
    // deliberately stops the gated services and closes the scheduler gate, so
    // restore the surviving key's runtime after that teardown has completed.
    if removed_session && api_key::has_api_key(config).await {
        start_credential_gated_services(ctx, config).await;
        crate::platform::llm_gate::set_signed_out(false);
        sentry_scope::clear();
        identity::clear_current_user();
        logs.push("credential-gated services restarted for api key".to_string());
    }

    if removed_session || removed_api_key {
        publish_credential_changed(ctx, "cleared");
    }
    Ok(Outcome::new(
        json!({
            "removed": removed_session || removed_api_key,
            "removedSession": removed_session,
            "removedApiKey": removed_api_key,
        }),
        logs,
    ))
}

/// Historical entry point: install a session (or local) credential from a
/// bare token. Same as [`set_credential`] with the kind classified by shape.
pub async fn store_session(
    ctx: &DispatchCtx,
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<Value>,
) -> Result<Outcome<AuthStateResponse>, String> {
    set_credential(
        ctx,
        config,
        SetCredentialRequest {
            token: token.to_string(),
            kind: None,
            user_id,
            user,
            issuing_backend: None,
        },
    )
    .await
}

/// Historical entry point: sign the session out. Same as
/// [`clear_credential`] for the session kind.
pub async fn clear_session(ctx: &DispatchCtx, config: &Config) -> Result<Outcome<Value>, String> {
    clear_credential(ctx, config, Some(CredentialKind::Session)).await
}
